
use super::*;
use crate::parse_source;

fn source() -> SourceIdentity {
    SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    )
}

fn validate_text(text: &str) -> ScalarValidation {
    let parsed = parse_source(source(), text.to_owned(), &[]);
    assert!(
        parsed.diagnostics.is_empty(),
        "syntax diagnostics: {:?}",
        parsed.diagnostics
    );
    derive_scalar_program(&parsed.result)
}

#[test]
fn source_aggregate_copy_policies_in_both_validators() {
    for (text, expected, invalid_modifier) in [
            (
                "%%start\nstruct Plain { i32 value; }\nstruct copy Point { i32 value; }\nenum copy Status { ok; }\n%%end",
                vec![CopyPolicy::Move, CopyPolicy::Copy],
                false,
            ),
            (
                "%%start\nstruct copy Inner { i32 value; }\nstruct copy Outer { Inner nested; Inner[2] values; }\n%%end",
                vec![CopyPolicy::Copy, CopyPolicy::Copy],
                false,
            ),
            (
                "%%start\nstruct Plain { i32 value; }\nstruct copy Invalid { Plain nested; }\n%%end",
                vec![CopyPolicy::Move, CopyPolicy::Move],
                true,
            ),
            (
                "%%start\nstruct copy Invalid { *!i32 value; }\n%%end",
                vec![CopyPolicy::Move],
                true,
            ),
        ] {
            let single = validate_text(text);
            let parsed = parse_source(source(), text.to_owned(), &[]);
            let module = ScalarModule::from_program(
                derive_scalar_program(&parsed.result).program,
                Vec::new(),
            );
            let project = validate_scalar_project(ScalarProject::new(vec![module], vec![source()]));
            for (structs, enums, diagnostics) in [
                (
                    &single.program.structs,
                    &single.program.enums,
                    &single.diagnostics,
                ),
                (
                    &project.project.modules[0].structs,
                    &project.project.modules[0].enums,
                    &project.diagnostics,
                ),
            ] {
                assert_eq!(
                    structs
                        .iter()
                        .map(|item| item.copy_policy)
                        .collect::<Vec<_>>(),
                    expected
                );
                if text.contains("enum copy") {
                    assert_eq!(enums[0].copy_policy, CopyPolicy::Copy);
                }
                let invalid = diagnostics
                    .iter()
                    .find(|item| item.code == "B0003" && item.message.contains("copy declaration"));
                if invalid_modifier {
                    assert_eq!(
                        invalid.unwrap().labels[0].span.range.start,
                        text.find("copy").unwrap() as u32
                    );
                } else {
                    assert!(invalid.is_none(), "{diagnostics:?}");
                }
            }
        }
}

#[test]
fn imported_aggregate_policies_and_moves_use_source_identity() {
    let library_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/library.w".into(),
        "r1".into(),
    );
    let library_text = "%%start\nstruct copy Copied { i32 value; }\nstruct Plain { i32 value; }\nenum Status { ok; }\n%%end";
    let main_text = "%%start\nlibrary = namespace package \"src/library.w\";\nstruct copy Wrapper { library.Copied value; }\nlibrary.Copied(library.Copied) copy_imported = fn(item) { item };\nlibrary.Plain(library.Plain) move_imported = fn(item) { item };\nlibrary.Status(library.Status) move_imported_enum = fn(item) { item };\n%%end";
    let main = parse_source(source(), main_text.to_owned(), &[]);
    let library = parse_source(library_source.clone(), library_text.to_owned(), &[]);
    assert!(main.diagnostics.is_empty(), "{:?}", main.diagnostics);
    assert!(library.diagnostics.is_empty(), "{:?}", library.diagnostics);
    let project = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                derive_scalar_program(&main.result).program,
                vec![ScalarNamespaceBinding {
                    binding: "library".to_owned(),
                    target: library_source.clone(),
                    span: ByteSpan::new(0, 0),
                }],
            ),
            ScalarModule::from_program(derive_scalar_program(&library.result).program, Vec::new()),
        ],
        vec![source(), library_source.clone()],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    assert_eq!(
        project.project.modules[0].structs[0].copy_policy,
        CopyPolicy::Copy
    );
    for item in &project.project.modules[0].items {
        let ScalarItem::Function(function) = item else {
            continue;
        };
        let moves = function
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Move { place, ty, owner } => Some((point, place, ty, owner)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if function.name == "copy_imported" {
            assert!(moves.is_empty(), "{moves:?}");
        } else {
            assert_eq!(moves.len(), 1, "{}: {moves:?}", function.name);
            let (point, place, ty, owner) = moves[0];
            assert_eq!(place.binding, *owner);
            assert_eq!(place.declaration_span, function.parameter_spans[0]);
            assert_eq!(
                point.source_span.range,
                scalar_place_target_span(&place.place)
            );
            assert_eq!(
                copy_policy(
                    ty,
                    &project.project.modules[1].structs,
                    &project.project.modules[1].enums,
                    &BTreeSet::new()
                ),
                Ok(CopyPolicy::Move)
            );
        }
    }
}

#[test]
fn generic_template_copy_policy_resolves_at_concrete_call_points_in_both_validators() {
    let text = "%%start\nstruct copy Point { i32 value; }\nstruct Payload { i32 value; }\nidentity = overload { generic T; T(T) => fn(value) { value }; };\nu8(u8) copy_byte = fn(value) { identity(value) };\nPoint(Point) copy_point = fn(value) { identity(value) };\nPayload(Payload) move_payload = fn(value) { identity(value) };\nu8[](u8[]) move_runtime = fn(value) { identity(value) };\n*u8(*u8) copy_shared = fn(value) { identity(value) };\n*!u8(*!u8) move_mutable = fn(value) { identity(value) };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(template) = &items[0] else {
            panic!("generic template")
        };
        let deferred = template.overload_arms[0]
            .reference_cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::DeferredMove { .. }))
            .expect("symbolic template event");
        assert!(!template.overload_arms[0]
            .reference_cfg
            .points
            .iter()
            .any(|point| matches!(point.kind, CfgPointKind::Move { .. })));
        for (function, expected) in items.iter().skip(1).zip([
            CopyPolicy::Copy,
            CopyPolicy::Copy,
            CopyPolicy::Move,
            CopyPolicy::Move,
            CopyPolicy::Copy,
            CopyPolicy::Move,
        ]) {
            let ScalarItem::Function(function) = function else {
                panic!("caller")
            };
            let call = function
                .reference_cfg
                .points
                .iter()
                .find(|point| {
                    matches!(
                        &point.kind,
                        CfgPointKind::Call {
                            output: 0,
                            target: ResolvedCallTarget::ModuleCallable { .. },
                            ..
                        }
                    )
                })
                .expect("concrete call");
            let CfgPointKind::Call {
                target:
                    ResolvedCallTarget::ModuleCallable {
                        source: target_source,
                        declaration_span,
                        concrete: ConcreteCallSelection::Overload(selection),
                    },
                ..
            } = &call.kind
            else {
                panic!("selected generic arm")
            };
            assert_eq!(target_source, &source());
            assert_eq!(*declaration_span, template.name_span);
            assert_eq!(selection.arm_index, 0);
            let specialized = function
                .reference_cfg
                .specialized_policies
                .iter()
                .find(|fact| fact.call_point == call.id)
                .expect("concrete policy at call point");
            assert_eq!(specialized.template_source, source());
            assert_eq!(
                specialized.template_function_span,
                template.overload_arms[0].span
            );
            assert_eq!(specialized.template_point, deferred.id);
            assert_eq!(specialized.policy, expected);
            assert_eq!(selection.substitutions.get("T"), Some(&specialized.ty));
            let moves = function.reference_cfg.points.iter().filter(|point| matches!(&point.kind, CfgPointKind::Move { owner, .. } if owner.declaration_span == function.parameter_spans[0])).collect::<Vec<_>>();
            assert_eq!(
                moves.len(),
                usize::from(expected == CopyPolicy::Move),
                "{}: {moves:?}",
                function.name
            );
            if let Some(moved) = moves.first() {
                assert_eq!(
                    moved.source_span.range.start,
                    call.source_span.range.start + "identity(".len() as u32
                );
            }
        }
    }
}

#[test]
fn imported_generic_arm_specialization_retains_declaring_source() {
    let library_source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/library.w".into(),
        "r1".into(),
    );
    let library = parse_source(
        library_source.clone(),
        "%%start\nidentity = overload { generic T; T(T) => fn(value) { value }; };\n%%end"
            .to_owned(),
        &[],
    );
    let main_text = "%%start\nlibrary = namespace package \"src/library.w\";\nstruct copy Point { i32 value; }\nstruct Payload { i32 value; }\nPoint(Point) copy_point = fn(value) { library.identity(value) };\nPayload(Payload) move_payload = fn(value) { library.identity(value) };\n%%end";
    let main = parse_source(source(), main_text.to_owned(), &[]);
    assert!(library.diagnostics.is_empty(), "{:?}", library.diagnostics);
    assert!(main.diagnostics.is_empty(), "{:?}", main.diagnostics);
    let project = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                derive_scalar_program(&main.result).program,
                vec![ScalarNamespaceBinding {
                    binding: "library".into(),
                    target: library_source.clone(),
                    span: ByteSpan::new(0, 0),
                }],
            ),
            ScalarModule::from_program(derive_scalar_program(&library.result).program, Vec::new()),
        ],
        vec![source(), library_source.clone()],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    let ScalarItem::Function(template) = &project.project.modules[1].items[0] else {
        panic!("template")
    };
    for (item, expected) in project.project.modules[0]
        .items
        .iter()
        .filter(|item| matches!(item, ScalarItem::Function(_)))
        .zip([CopyPolicy::Copy, CopyPolicy::Move])
    {
        let ScalarItem::Function(function) = item else {
            unreachable!()
        };
        let call = function
            .reference_cfg
            .points
            .iter()
            .find(|point| {
                matches!(
                    &point.kind,
                    CfgPointKind::Call {
                        output: 0,
                        target: ResolvedCallTarget::ModuleCallable { .. },
                        ..
                    }
                )
            })
            .expect("qualified call");
        let CfgPointKind::Call {
            target:
                ResolvedCallTarget::ModuleCallable {
                    source: target_source,
                    declaration_span,
                    concrete: ConcreteCallSelection::Overload(selection),
                },
            ..
        } = &call.kind
        else {
            panic!("selected arm")
        };
        assert_eq!(target_source, &library_source);
        assert_eq!(*declaration_span, template.name_span);
        assert_eq!(selection.arm_index, 0);
        let fact = function
            .reference_cfg
            .specialized_policies
            .iter()
            .find(|fact| fact.call_point == call.id)
            .expect("specialized policy");
        assert_eq!(fact.template_source, library_source);
        assert_eq!(fact.template_function_span, template.overload_arms[0].span);
        assert_eq!(fact.policy, expected);
        assert_eq!(selection.substitutions.get("T"), Some(&fact.ty));
        assert_eq!(
            function
                .reference_cfg
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::Move { .. }))
                .count(),
            usize::from(expected == CopyPolicy::Move)
        );
    }
}

#[test]
fn explicitly_instantiated_generic_declaration_has_concrete_copy_policy() {
    let text = "%%start\ngeneric T;\nT(T) identity = fn(value) { value };\nu8(u8) copy_byte = fn(value) { identity<u8>(value) };\nu8[](u8[]) move_runtime = fn(value) { identity<u8[]>(value) };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(template) = &items[0] else {
            panic!("template")
        };
        assert!(template
            .reference_cfg
            .points
            .iter()
            .any(|point| matches!(point.kind, CfgPointKind::DeferredMove { .. })));
        for (item, expected) in items
            .iter()
            .skip(1)
            .zip([CopyPolicy::Copy, CopyPolicy::Move])
        {
            let ScalarItem::Function(function) = item else {
                panic!("caller")
            };
            let call = function
                .reference_cfg
                .points
                .iter()
                .find(|point| {
                    matches!(
                        &point.kind,
                        CfgPointKind::Call {
                            output: 0,
                            target: ResolvedCallTarget::ModuleCallable { .. },
                            ..
                        }
                    )
                })
                .expect("generic call");
            let fact = function
                .reference_cfg
                .specialized_policies
                .iter()
                .find(|fact| fact.call_point == call.id)
                .expect("specialized policy");
            assert_eq!(fact.policy, expected);
            assert_eq!(fact.template_function_span, template.span);
            assert_eq!(fact.template_source, source());
        }
    }
}

#[test]
fn generic_array_element_policy_defers_until_concrete_call() {
    let text = "%%start\nstruct Payload { i32 value; }\npick = overload { generic T; T(T[2], u64) => fn(values, index) { values[index] }; };\nu8(u8[2], u64) copy_byte = fn(values, index) { pick(values, index) };\nPayload(Payload[2], u64) move_payload = fn(values, index) { pick(values, index) };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == "array elements cannot be moved individually")
            .expect("specialized array guard");
        assert_eq!(
            diagnostic.labels[0].span.range.start,
            text.find("pick(values, index) };\n%%end").unwrap() as u32
        );
    }
}

#[test]
fn unresolved_copy_member_preserves_original_diagnostic_and_required_policy_errors() {
    let text = "%%start\nstruct copy Broken { Missing value; }\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    assert!(
        !single.diagnostics.is_empty(),
        "single: {:?}",
        single.program.structs[0].fields
    );
    assert!(
        !project.diagnostics.is_empty(),
        "project: {:?}",
        project.project.modules[0].structs[0].fields
    );
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic
                    .labels
                    .iter()
                    .any(|label| label.span.range.start == text.find("Missing").unwrap() as u32)),
            "{diagnostics:?}"
        );
        assert!(
            !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("copy declaration")),
            "{diagnostics:?}"
        );
    }
    assert!(copy_policy(&ScalarType::Error, &[], &[], &BTreeSet::new()).is_err());
    assert!(copy_policy(
        &ScalarType::Struct(single.program.structs[0].id.clone()),
        &[],
        &[],
        &BTreeSet::new()
    )
    .is_err());
}

#[test]
fn missing_aggregate_policy_table_reports_spanned_invariant() {
    let text = "%%start\nstruct Payload { i32 value; }\nPayload(Payload) take = fn(value) { value };\n%%end";
    let validated = validate_text(text);
    assert!(
        validated.diagnostics.is_empty(),
        "{:?}",
        validated.diagnostics
    );
    let ScalarItem::Function(function) = &validated.program.items[0] else {
        panic!("function")
    };
    let mut function = function.clone();
    let read_span = expression_span(&function.body.expressions[0]);
    let errors = record_function_reference_origins(
        &validated.program.source,
        &mut function,
        &HashSet::new(),
        &[],
        &[],
        &[],
        &HashMap::new(),
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(
        errors[0].reason(),
        ReferenceAnalysisErrorReason::MissingResolvedBinding
    );
    assert_eq!(
        errors[0].source_span(),
        &SourceSpan::new(validated.program.source, read_span)
    );
}

#[test]
fn mutable_checked_dereference_of_move_only_pointee_is_a_typed_move() {
    let text = "%%start\nstruct Payload { i32 value; }\nPayload(Payload) take = fn(value) { *!Payload writer = &!value; Payload moved = *writer; moved };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let event = function
            .reference_cfg
            .points
            .iter()
            .find(|point| {
                matches!(
                    &point.kind,
                    CfgPointKind::MoveThrough {
                        place: ScalarPlace::Dereference { .. },
                        ..
                    }
                )
            })
            .expect("whole aggregate move");
        let CfgPointKind::MoveThrough { candidates, ty, .. } = &event.kind else {
            unreachable!()
        };
        assert!(matches!(ty, ScalarType::Struct(_)));
        assert_eq!(candidates.len(), 1);
        assert_eq!(event.possible_origins.as_ref().map(BTreeSet::len), Some(1));
        assert_eq!(
            candidates[0].place.binding.declaration_span,
            function.parameter_spans[0]
        );
        assert!(
            matches!(&candidates[0].place.projections[..], [ReferencePlaceProjection::Dereference(loan)] if loan.creation_span.start == text.find("&!value").unwrap() as u32)
        );
        let start = text.find("*writer; moved").unwrap() as u32;
        assert_eq!(
            event.source_span.range,
            ByteSpan::new(start, start + "*writer".len() as u32)
        );
    }
}

#[test]
fn whole_checked_aggregate_reads_preserve_source_places_in_both_validators() {
    let text = "%%start\nstruct copy Point { u8 value; }\nstruct Payload { u8 value; }\nunit(Point, u8[2], Payload, Payload[2]) inspect = fn(item, bytes, value, items) { *Point reader = &item; Point first = *reader; Point second = *reader; u8 field = (*reader).value; *(u8[2]) array_reader = &bytes; u8[2] array = *array_reader; u8 element = array[0]; *!Payload writer = &!value; u8 prior = (*writer).value; Payload taken = *writer; *!(Payload[2]) array_writer = &!items; Payload[2] moved_array = *array_writer; first; second; field; element; prior; taken; moved_array; } ;\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let cfg = &function.reference_cfg;
        for (spelling, parameter) in [
            ("*reader;", 0),
            ("(*reader).value", 0),
            ("*array_reader;", 1),
            ("(*writer).value", 2),
        ] {
            let start = text.find(spelling).unwrap() as u32;
            let reads = cfg
                .points
                .iter()
                .filter(|point| {
                    point.source_span.range.start == start
                        && matches!(
                            &point.kind,
                            CfgPointKind::ReadPlace {
                                reads_origin: false,
                                ..
                            }
                        )
                })
                .collect::<Vec<_>>();
            assert!(!reads.is_empty(), "{spelling}: {:?}", cfg.points);
            for read in reads {
                let CfgPointKind::ReadPlace { candidates, .. } = &read.kind else {
                    unreachable!()
                };
                assert_eq!(candidates.len(), 1, "{spelling}");
                assert_eq!(
                    candidates[0].binding.declaration_span,
                    function.parameter_spans[parameter]
                );
                assert!(matches!(
                    candidates[0].projections.first(),
                    Some(ReferencePlaceProjection::Dereference(_))
                ));
                assert_eq!(read.possible_origins.as_ref().map(BTreeSet::len), Some(1));
            }
        }
        for (spelling, parameter) in [("*writer;", 2), ("*array_writer;", 3)] {
            let start = text.find(spelling).unwrap() as u32;
            let event = cfg
                .points
                .iter()
                .find(|point| {
                    point.source_span.range.start == start
                        && matches!(
                            &point.kind,
                            CfgPointKind::MoveThrough {
                                place: ScalarPlace::Dereference { .. },
                                ..
                            }
                        )
                })
                .expect("whole move");
            let CfgPointKind::MoveThrough { candidates, ty, .. } = &event.kind else {
                unreachable!()
            };
            assert!(matches!(
                ty,
                ScalarType::Struct(_) | ScalarType::Array { .. }
            ));
            assert_eq!(candidates.len(), 1);
            assert_eq!(
                candidates[0].place.binding.declaration_span,
                function.parameter_spans[parameter]
            );
            assert!(matches!(
                candidates[0].place.projections.first(),
                Some(ReferencePlaceProjection::Dereference(_))
            ));
            assert_eq!(event.possible_origins.as_ref().map(BTreeSet::len), Some(1));
        }
        assert!(!cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::MoveThrough { place: ScalarPlace::Dereference { pointer, .. }, .. } if matches!(pointer.as_ref(), ScalarExpression::Name { name, .. } if name == "reader" || name == "array_reader"))));
    }
}

#[test]
fn whole_checked_aggregate_move_keeps_joined_owner_candidates() {
    let text = "%%start\nstruct Payload { u8 value; }\nPayload(*!Payload, *!Payload, bool) choose = fn(first, second, flag) { *!Payload writer = first; if (flag) { writer = first; } else { writer = second; }; Payload moved = *writer; moved };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let event = function
            .reference_cfg
            .points
            .iter()
            .find(|point| {
                matches!(
                    &point.kind,
                    CfgPointKind::MoveThrough {
                        place: ScalarPlace::Dereference { .. },
                        ..
                    }
                )
            })
            .expect("joined whole move");
        let CfgPointKind::MoveThrough { candidates, ty, .. } = &event.kind else {
            unreachable!()
        };
        assert!(matches!(ty, ScalarType::Struct(_)));
        assert_eq!(
            event.source_span.range.start,
            text.find("*writer; moved").unwrap() as u32
        );
        assert_eq!(event.possible_origins.as_ref().map(BTreeSet::len), Some(2));
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.place.binding.declaration_span)
                .collect::<HashSet<_>>(),
            HashSet::from([function.parameter_spans[0], function.parameter_spans[1]])
        );
        assert!(candidates.iter().all(|candidate| matches!(candidate.place.projections.first(), Some(ReferencePlaceProjection::Dereference(loan)) if loan.as_ref() == &candidate.loan)));
    }
}

#[test]
fn checked_aggregate_edge_availability_in_both_validators() {
    let cases = [
            ("struct copy Point { u8 value; } unit(Point) f = fn(p) { *Point reader = &p; Point a = *reader; Point b = *reader; u8 field = (*reader).value; a; b; field; };", None),
            ("unit(u8[2]) f = fn(bytes) { *(u8[2]) reader = &bytes; u8[2] a = *reader; u8[2] b = *reader; u8 element = b[0]; a; element; };", None),
            ("struct Payload { u8 value; } unit(Payload) f = fn(p) { *!Payload writer = &!p; u8 before = (*writer).value; Payload taken = *writer; before; taken; };", None),
            ("struct Payload { u8 value; } unit(Payload) f = fn(p) { *!Payload writer = &!p; Payload taken = *writer; Payload again = *writer; taken; again; };", Some("*writer; taken")),
            ("struct Payload { u8 value; } unit(Payload) f = fn(p) { *!Payload writer = &!p; Payload taken = *writer; u8 after = (*writer).value; taken; after; };", Some("(*writer).value")),
            ("struct Payload { u8 value; } unit(Payload[2]) f = fn(items) { *!(Payload[2]) writer = &!items; Payload[2] taken = *writer; taken; };", None),
            ("struct Payload { u8 value; } unit(Payload[2]) f = fn(items) { *!(Payload[2]) writer = &!items; Payload[2] taken = *writer; Payload[2] again = *writer; taken; again; };", Some("*writer; taken")),
            ("struct Payload { u8 value; } struct Pair { Payload left; Payload right; } unit(Pair) f = fn(source) { Payload left = source.left; Payload right = source.right; left; right; };", None),
            ("struct Payload { u8 value; } struct Pair { Payload left; Payload right; } unit(Pair) f = fn(source) { Payload left = source.left; Pair again = source; left; again; };", Some("source; left")),
            ("struct Payload { u8 value; } unit(Payload) f = fn(source) { *Payload retained = &source; Payload moved = source; retained; moved; };", Some("source; retained")),
            ("struct Payload { u8 value; } struct Pair { Payload left; Payload right; } unit(Pair) f = fn(source) { *Payload retained = &source.right; Payload taken = source.left; retained; taken; };", None),
            ("unit(u64) f = fn(index) { u8[2] bytes = [1, 2]; *u8 retained = &bytes[1]; index; bytes[index] = 3; retained; };", Some("retained;")),
        ];
    for (body, conflicting) in cases {
        let text = format!(
            "%%start\n{}\n%%end",
            body.replace("} unit(", "}\nunit(")
                .replace("} struct ", "}\nstruct ")
        );
        let single = validate_text(&text);
        let parsed = parse_source(source(), text.clone(), &[]);
        assert!(
            parsed.diagnostics.is_empty(),
            "{body}: {:?}",
            parsed.diagnostics
        );
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(
                derive_scalar_program(&parsed.result).program,
                Vec::new(),
            )],
            vec![source()],
        ));
        for diagnostics in [&single.diagnostics, &project.diagnostics] {
            match conflicting {
                Some(spelling) => {
                    let start = text.find(spelling).expect("conflict spelling") as u32;
                    assert!(
                        diagnostics.iter().any(|diagnostic| {
                            diagnostic.code == "B0003"
                                && (diagnostic.message == "read or move of unavailable place"
                                    || (spelling == "retained;"
                                        && diagnostic.message
                                            == "missing checked-reference origin"))
                                && diagnostic.labels[0].span.range.start == start
                        }),
                        "{body}: {diagnostics:?}"
                    );
                }
                None => assert!(diagnostics.is_empty(), "{body}: {diagnostics:?}"),
            }
        }
    }
}

#[test]
fn specialized_checked_aggregate_reads_transfer_deferred_policy() {
    let text = "%%start\nstruct copy Point { u8 value; }\nstruct Payload { u8 value; }\nread_twice = overload { generic T; T(T) => fn(value) { *!T writer = &!value; T first = *writer; T second = *writer; first; second }; };\nPoint(Point) copied = fn(value) { read_twice(value) };\nPayload(Payload) moved = fn(value) { read_twice(value) };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        let ScalarItem::Function(template) = &items[0] else {
            panic!("template")
        };
        assert!(template.overload_arms[0]
            .reference_cfg
            .points
            .iter()
            .any(|point| matches!(point.kind, CfgPointKind::DeferredMoveThrough { .. })));
        let copy_call = text.find("read_twice(value) }").unwrap() as u32;
        let move_call = text.rfind("read_twice(value) }").unwrap() as u32;
        assert!(
            !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.labels[0].span.range.start == copy_call),
            "{diagnostics:?}"
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.message == "read or move of unavailable place"
                    && diagnostic.labels[0].span.range.start == move_call),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn whole_copyable_checked_aggregate_null_keeps_existing_guard_policy() {
    let text = "%%start\nstruct copy Point { u8 value; }\nPoint() take = fn { *Point reader = null; Point result = *reader; result };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn checked_dereference_field_move_uses_parent_loan_and_original_owner() {
    let text = "%%start\nstruct Payload { i32 value; }\nstruct Holder { Payload nested; }\nPayload(Holder) take = fn(value) { *!Holder writer = &!value; Payload moved = (*writer).nested; moved };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    for items in [&single.program.items, &project.project.modules[0].items] {
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let moves = function
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Move { place, ty, owner } if !place.projections.is_empty() => {
                    Some((point, place, ty, owner))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(moves.len(), 1, "{moves:?}");
        let (point, place, ty, owner) = moves[0];
        assert!(matches!(ty, ScalarType::Struct(_)));
        assert_eq!(place.binding, *owner);
        assert_eq!(owner.declaration_span, function.parameter_spans[0]);
        assert_eq!(
            point.source_span.range.start,
            text.find("(*writer).nested").unwrap() as u32
        );
        assert!(
            matches!(&place.projections[..], [ReferencePlaceProjection::Dereference(loan), ReferencePlaceProjection::Field(_)] if loan.origin_place.binding == *owner && loan.mode == ScalarReferenceMutability::Mutable)
        );
    }
}

#[test]
fn joined_checked_dereference_move_preserves_both_parent_loans() {
    let text = "%%start\nstruct Payload { i32 value; }\nstruct Holder { Payload nested; }\nPayload(*!Holder, *!Holder, bool) choose = fn(first, second, flag) { *!Holder writer = first; if (flag) { writer = first; } else { writer = second; }; Payload moved = (*writer).nested; moved };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let event = function
            .reference_cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::MoveThrough { .. }))
            .expect("joined move");
        let CfgPointKind::MoveThrough { candidates, ty, .. } = &event.kind else {
            unreachable!()
        };
        assert!(matches!(ty, ScalarType::Struct(_)));
        assert_eq!(candidates.len(), 2, "{candidates:?}");
        assert_eq!(event.possible_origins.as_ref().map(BTreeSet::len), Some(2));
        assert_eq!(
            event.source_span.range.start,
            text.find("(*writer).nested").unwrap() as u32
        );
        let owners = candidates
            .iter()
            .map(|candidate| candidate.place.binding.declaration_span)
            .collect::<HashSet<_>>();
        assert_eq!(
            owners,
            HashSet::from([function.parameter_spans[0], function.parameter_spans[1]])
        );
        for candidate in candidates {
            assert!(
                matches!(&candidate.place.projections[..], [ReferencePlaceProjection::Dereference(loan), ReferencePlaceProjection::Field(_)] if loan.as_ref() == &candidate.loan)
            );
            assert!(event
                .possible_origins
                .as_ref()
                .unwrap()
                .contains(&candidate.origin));
        }
    }
}

#[test]
fn copyable_checked_dereference_remains_a_read_in_both_validators() {
    let text = "%%start\nstruct copy Point { i32 value; }\ni32(Point) take = fn(value) { *Point reader = &value; (*reader).value };\ni32(i32) read_scalar = fn(value) { *i32 reader = &value; *reader };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        for item in items {
            let ScalarItem::Function(function) = item else {
                continue;
            };
            assert!(!function.reference_cfg.points.iter().any(|point| matches!(
                point.kind,
                CfgPointKind::Move { .. } | CfgPointKind::MoveThrough { .. }
            )));
            assert!(function.reference_cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Read { binding } if binding.declaration_span == function.body.items.iter().find_map(|item| match item { ScalarBlockItem::LocalBinding(binding) => Some(binding.name_span), _ => None }).unwrap())));
        }
    }
}

#[test]
fn indexed_dereference_of_fixed_array_is_rejected_by_current_grammar() {
    let text = "%%start\nstruct Payload { i32 value; }\nPayload(Payload[2], u64) take = fn(values, index) { *!(Payload[2]) writer = &!values; (*writer)[index] };\n%%end";
    let parsed = parse_source(source(), text.to_owned(), &[]);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "S0001"),
        "{:?}",
        parsed.diagnostics
    );
}

#[test]
fn typed_move_events_from_source_in_both_validators() {
    let text = "%%start\nstruct Payload { i32 value; }\nstruct Holder { Payload nested; }\nstruct copy Point { i32 value; }\nenum DefaultStatus { ok; }\nenum copy CopyStatus { ok; }\nPayload(Payload) move_payload = fn(value) { value };\nPayload(Holder) move_field = fn(container) { container.nested };\nPoint(Point) copy_point = fn(item) { item };\nDefaultStatus(DefaultStatus) move_enum = fn(item) { item };\nCopyStatus(CopyStatus) copy_enum = fn(item) { item };\nPayload[2](Payload[2]) move_fixed = fn(values) { values };\ni32[2](i32[2]) copy_fixed = fn(values) { values };\nu8[](u8[]) move_runtime = fn(values) { values };\n*!i32(*!i32) move_mutable = fn(value) { value };\ni32(Payload) read_only = fn(value) { value.value };\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let module =
        ScalarModule::from_program(derive_scalar_program(&parsed.result).program, Vec::new());
    let project = validate_scalar_project(ScalarProject::new(vec![module], vec![source()]));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    for items in [&single.program.items, &project.project.modules[0].items] {
        for item in items {
            let ScalarItem::Function(function) = item else {
                continue;
            };
            let moves = function
                .reference_cfg
                .points
                .iter()
                .filter_map(|point| match &point.kind {
                    CfgPointKind::Move { place, ty, owner } => Some((point, place, ty, owner)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            match function.name.as_str() {
                "copy_point" | "copy_fixed" | "copy_enum" | "read_only" => {
                    assert!(moves.is_empty(), "{}: {moves:?}", function.name)
                }
                _ => {
                    assert_eq!(moves.len(), 1, "{}: {moves:?}", function.name);
                    let (point, place, ty, owner) = moves[0];
                    assert_eq!(&place.binding, owner);
                    assert_eq!(place.binding.declaration_span, function.parameter_spans[0]);
                    assert_eq!(
                        point.source_span.range,
                        scalar_place_target_span(&place.place)
                    );
                    assert_eq!(
                        copy_policy(
                            ty,
                            &single.program.structs,
                            &single.program.enums,
                            &BTreeSet::new()
                        ),
                        Ok(CopyPolicy::Move)
                    );
                    if function.name == "move_field" {
                        assert!(
                            matches!(&place.projections[..], [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field))] if field.index == 0 && field.structure == single.program.structs[1].id)
                        );
                    } else {
                        assert!(place.projections.is_empty());
                    }
                }
            }
        }
    }
}

#[test]
fn indexed_aggregate_element_move_is_rejected_in_both_validators() {
    let text = "%%start\nstruct Payload { i32 value; }\nPayload(Payload[2], u64) take = fn(values, index) { values[index] };\n%%end";
    let single = validate_text(text);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == "array elements cannot be moved individually")
            .expect("individual array move guard");
        assert_eq!(
            diagnostic.labels[0].span.range.start,
            text.find("values[index]").unwrap() as u32
        );
    }
}

#[test]
fn inferred_bindings_preserve_resolved_copy_and_move_types_in_both_validators() {
    let text = "%%start\nstruct Payload { i32 value; }\nPayload(Payload, Payload) restore = fn(first, second) { next = first; saved = next; saved; next = second; next };\nu8[](u8[]) transfer = fn(values) { next = values; next };\ni32(i32) copy_value = fn(value) { next = value; next + value };\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    for items in [&single.program.items, &project.project.modules[0].items] {
        for item in items {
            let ScalarItem::Function(function) = item else {
                continue;
            };
            let moves = function
                .reference_cfg
                .points
                .iter()
                .filter_map(|point| match &point.kind {
                    CfgPointKind::Move { place, ty, owner } => Some((point, place, ty, owner)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            match function.name.as_str() {
                "copy_value" => assert!(moves.is_empty(), "{moves:?}"),
                "transfer" => {
                    assert_eq!(moves.len(), 2, "{moves:?}");
                    assert_eq!(
                        moves[0].1.binding.declaration_span,
                        function.parameter_spans[0]
                    );
                    let ScalarBlockItem::Assignment(assignment) = &function.body.items[0] else {
                        panic!("inferred assignment")
                    };
                    assert_eq!(
                        moves[1].1.binding.declaration_span,
                        assignment.targets[0].target_span
                    );
                }
                "restore" => {
                    assert_eq!(moves.len(), 5, "{moves:?}");
                    assert_eq!(
                        moves[1].1.binding.declaration_span,
                        moves[4].1.binding.declaration_span
                    );
                }
                _ => unreachable!(),
            }
            for (point, place, ty, owner) in moves {
                assert_eq!(&place.binding, owner);
                assert_eq!(
                    point.source_span.range,
                    scalar_place_target_span(&place.place)
                );
                assert_eq!(
                    copy_policy(
                        ty,
                        &single.program.structs,
                        &single.program.enums,
                        &BTreeSet::new()
                    ),
                    Ok(CopyPolicy::Move)
                );
            }
        }
    }
}

#[test]
fn inferred_multi_output_call_keeps_each_output_type_and_owner() {
    let text = "%%start\nstruct Payload { i32 value; }\n(Payload, u8[])(Payload, u8[]) pair = fn(left, right) { left, right };\nPayload(Payload, u8[]) consume = fn(left, right) { first, second = pair(left, right); second; first };\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let parsed = parse_source(source(), text.to_owned(), &[]);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            derive_scalar_program(&parsed.result).program,
            Vec::new(),
        )],
        vec![source()],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    for (items, facts) in [
        (
            &single.program.items,
            &single.program.resolved_assignment_outputs,
        ),
        (
            &project.project.modules[0].items,
            &project.project.modules[0].resolved_assignment_outputs,
        ),
    ] {
        let ScalarItem::Function(function) = &items[1] else {
            panic!("consumer")
        };
        let ScalarBlockItem::Assignment(assignment) = &function.body.items[0] else {
            panic!("inferred outputs")
        };
        for (output_index, target) in assignment.targets.iter().enumerate() {
            let key = ResolvedAssignmentOutputId {
                source: source(),
                function_span: function.span,
                target_span: target.target_span,
                output_index,
            };
            let facts = facts.borrow();
            let ty = facts.get(&key).expect("resolved output fact");
            if output_index == 0 {
                assert!(matches!(ty, ScalarType::Struct(_)));
            } else {
                assert!(matches!(ty, ScalarType::RuntimeArray { .. }));
            }
        }
        let moves = function
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Move { place, ty, owner } => Some((place, ty, owner)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(moves.len(), 4, "{moves:?}");
        assert_eq!(
            moves[2].0.binding.declaration_span,
            assignment.targets[1].target_span
        );
        assert!(matches!(moves[2].1, ScalarType::RuntimeArray { .. }));
        assert_eq!(
            moves[3].0.binding.declaration_span,
            assignment.targets[0].target_span
        );
        assert!(matches!(moves[3].1, ScalarType::Struct(_)));
        assert_eq!(&moves[2].0.binding, moves[2].2);
        assert_eq!(&moves[3].0.binding, moves[3].2);
    }
    let serialized = serde_json::to_value(&project.project).expect("serialized project");
    let restored: ScalarProject = serde_json::from_value(serialized).expect("restored project");
    assert!(restored.modules[0]
        .resolved_assignment_outputs
        .borrow()
        .is_empty());
    let revalidated = validate_scalar_project(restored);
    assert!(
        revalidated.diagnostics.is_empty(),
        "{:?}",
        revalidated.diagnostics
    );
    assert_eq!(
        revalidated.project.modules[0]
            .resolved_assignment_outputs
            .borrow()
            .len(),
        project.project.modules[0]
            .resolved_assignment_outputs
            .borrow()
            .len()
    );
}

#[test]
fn absent_resolved_assignment_output_is_a_spanned_invariant_error() {
    let text = "%%start\nu8[](u8[]) transfer = fn(values) { next = values; next };\n%%end";
    let validated = validate_text(text);
    assert!(
        validated.diagnostics.is_empty(),
        "{:?}",
        validated.diagnostics
    );
    let ScalarItem::Function(function) = &validated.program.items[0] else {
        panic!("function")
    };
    let mut function = function.clone();
    let ScalarBlockItem::Assignment(assignment) = &function.body.items[0] else {
        panic!("assignment")
    };
    let target_span = assignment.targets[0].target_span;
    let errors = record_function_reference_origins(
        &validated.program.source,
        &mut function,
        &HashSet::new(),
        &[],
        &validated.program.structs,
        &validated.program.enums,
        &HashMap::new(),
    );
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].reason(),
        ReferenceAnalysisErrorReason::MissingResolvedBinding
    );
    assert_eq!(
        errors[0].source_span(),
        &SourceSpan::new(validated.program.source, target_span)
    );
}

#[test]
fn terminal_unsafe_block_derives_one_final_output() {
    for (source_text, final_expression, item_count) in [
        (
            "%%start\n*u8(u8) f = fn(v) { unsafe { &v } };\n%%end",
            "unsafe { &v }",
            1,
        ),
        (
            "%%start\n*u8(u8) f = fn(v) { unsafe { unsafe { &v } } };\n%%end",
            "unsafe { unsafe { &v } }",
            1,
        ),
        (
            "%%start\n*u8(u8) f = fn(v) { unsafe { &v } unsafe { &v } };\n%%end",
            "unsafe { &v }",
            2,
        ),
    ] {
        let result = validate_text(source_text);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let module_source = module_source("src/main.w");
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                module_source.clone(),
                module_from_text(module_source.clone(), source_text).items,
                Vec::new(),
            )],
            vec![module_source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for items in [&result.program.items, &project.project.modules[0].items] {
            let ScalarItem::Function(function) = &items[0] else {
                panic!("expected function");
            };
            let body = &function.body;
            assert_eq!(body.final_output_values.len(), 1);
            assert_eq!(body.items.len(), item_count);
            assert_eq!(body.terminated_items, vec![false; item_count]);
            assert_eq!(body.expressions.len(), item_count);
            let output = &body.final_output_values[0];
            assert_eq!(output.position, 0);
            let start = source_text.rfind(final_expression).expect("final unsafe") as u32;
            assert_eq!(
                output.span,
                ByteSpan::new(start, start + final_expression.len() as u32)
            );
            assert_eq!(
                body.items.last(),
                Some(&ScalarBlockItem::Expression(output.value.clone()))
            );
            assert!(matches!(
                &output.value,
                ScalarExpression::Block(block)
                    if block.unsafe_context && block.final_output_values.len() == 1
            ));
        }
    }
}

fn both_reference_functions(text: &str) -> (ScalarValidation, ScalarProjectValidation) {
    let single = validate_text(text);
    let source = module_source("src/main.w");
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            module_from_text(source.clone(), text),
            Vec::new(),
        )],
        vec![source],
    ));
    (single, project)
}

#[test]
fn loop_local_move_state_ends_with_its_scope_in_both_validators() {
    let text = "%%start\nstruct Payload { u64 length; }\nu64(Payload) parse = fn(text) { text.length };\nunit() run = fn { bool running = true; while (running) { Payload line = { .length = 1; }; u64 value = parse(line); value; running = false; } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let function = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "run" => Some(function),
                _ => None,
            })
            .expect("run function");
        let moved = function.reference_cfg.points.iter().find(|point| matches!(&point.kind, CfgPointKind::Move { place, .. } if place.binding.declaration_span.start == text.find("line =").unwrap() as u32)).expect("typed argument move");
        assert_eq!(
            moved.source_span.range.start,
            text.find("parse(line)").unwrap() as u32 + "parse(".len() as u32
        );
    }
}

#[test]
fn move_only_argument_to_multi_output_call_is_consumed_once_in_both_validators() {
    let text = "%%start\nstruct Payload { u64 length; }\n(u64, bool)(Payload) inspect = fn(text) { text.length, true };\nunit() run = fn { Payload line = { .length = 1; }; u64 size = 0; bool valid = false; size, valid = inspect(line); size; valid; u64 again = line.length; again; };\n%%end";
    let (single, project) = both_reference_functions(text);
    let first = text.find("inspect(line)").unwrap() as u32 + "inspect(".len() as u32;
    let second = text.find("line.length; again").unwrap() as u32;
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        let function = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "run" => Some(function),
                _ => None,
            })
            .expect("run function");
        assert_eq!(
            function
                .reference_cfg
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::Move { .. })
                    && point.source_span.range.start == first)
                .count(),
            1
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start == second),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn inner_scope_exit_preserves_outer_owner_move_in_both_validators() {
    let text = "%%start\nstruct Payload { u64 length; }\nunit() run = fn { Payload outer = { .length = 1; }; if (true) { Payload moved = outer; moved.length; }; outer.length; };\n%%end";
    let (single, project) = both_reference_functions(text);
    let read = text.rfind("outer.length").unwrap() as u32;
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start == read),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn checked_aggregate_conflicts_label_the_initiating_loan_in_both_pipelines() {
    for (text, primary, origin) in [
            ("%%start\nstruct Payload { u8 value; }\nunit() f = fn { Payload specimen = { .value = 1; }; *!Payload writer = &!specimen; Payload taken = *writer; Payload again = *writer; taken; again; };\n%%end", "*writer; taken", "&!specimen"),
            ("%%start\nstruct Payload { u8 value; }\nunit() f = fn { Payload specimen = { .value = 1; }; *Payload retained = &specimen; Payload moved = specimen; retained; moved; };\n%%end", "= specimen; retained", "&specimen"),
            ("%%start\nstruct Payload { u8 value; }\nstruct Holder { Payload nested; u8 other; }\nunit() f = fn { Holder specimen = { .nested = { .value = 1; }; .other = 2; }; *!Holder reader = &!specimen; Payload taken = specimen.nested; Holder whole = *reader; taken; whole; };\n%%end", "*reader; taken", "specimen.nested; Holder"),
            ("%%start\nunit(u64) f = fn(length) { u8[length] bytes; *u8 borrowed = &bytes[0]; u8[] moved = bytes; borrowed; moved; };\n%%end", "= bytes; borrowed", "&bytes[0]"),
            ("%%start\nunit(u64) f = fn(length) { u8[length] bytes; *u8 borrowed = &bytes[0]; unsafe { core.free(&bytes); }; borrowed; };\n%%end", "core.free(&bytes)", "&bytes[0]"),
        ] {
            let (single, project) = both_reference_functions(text);
            for (diagnostics, source) in [
                (&single.diagnostics, &single.program.source),
                (&project.diagnostics, &project.project.modules[0].source),
            ] {
                let primary_start = text.find(primary).unwrap() as u32 + if primary.starts_with("= ") { 2 } else { 0 };
                let diagnostic = diagnostics.iter().find(|diagnostic| diagnostic.code == "B0003" && diagnostic.message == "read or move of unavailable place" && diagnostic.labels[0].span.range.start == primary_start).unwrap_or_else(|| panic!("{text}: {diagnostics:?}"));
                assert_eq!(&diagnostic.labels[0].span.source, source);
                let secondary = diagnostic.labels.iter().find(|label| label.kind == super::super::DiagnosticLabelKind::Secondary && label.span.range.start == text.find(origin).unwrap() as u32).unwrap_or_else(|| panic!("{text}: {diagnostic:?}"));
                assert_eq!(&secondary.span.source, source);
            }
        }
}

#[test]
fn checked_aggregate_copy_and_single_move_are_accepted_in_both_pipelines() {
    for text in [
            "%%start\nstruct copy Point { u8 value; }\nunit() f = fn { Point sample = { .value = 1; }; *Point reader = &sample; Point a = *reader; Point b = *reader; a.value; b.value; };\n%%end",
            "%%start\nstruct Payload { u8 value; }\nunit() f = fn { Payload specimen = { .value = 1; }; *!Payload writer = &!specimen; u8 prior = (*writer).value; Payload taken = *writer; prior; taken.value; };\n%%end",
            "%%start\nunit() f = fn { u8[2] bytes = [1, 2]; *(u8[2]) reader = &bytes; u8[2] copy = *reader; copy[0]; bytes[1]; };\n%%end",
        ] {
            let (single, project) = both_reference_functions(text);
            assert!(single.diagnostics.is_empty(), "{text}: {:?}", single.diagnostics);
            assert!(project.diagnostics.is_empty(), "{text}: {:?}", project.diagnostics);
        }
}

#[test]
fn dynamic_index_write_reports_overlapping_loan_in_both_pipelines() {
    let text = "%%start\nunit(u64) f = fn(index) { u8[2] bytes = [1, 2]; *u8 reader = &bytes[0]; index; bytes[index] = 3; reader; };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (diagnostics, source) in [
        (&single.diagnostics, &single.program.source),
        (&project.diagnostics, &project.project.modules[0].source),
    ] {
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start
                        == text.find("bytes[index] =").unwrap() as u32 + "bytes[".len() as u32
            })
            .unwrap_or_else(|| panic!("{diagnostics:?}"));
        assert_eq!(&diagnostic.labels[0].span.source, source);
        assert!(
            diagnostic.labels.iter().any(|label| label.kind
                == super::super::DiagnosticLabelKind::Secondary
                && label.span.source == *source
                && label.span.range.start == text.find("&bytes[0]").unwrap() as u32),
            "{diagnostic:?}"
        );
    }
}

fn sole_fact<'a>(function: &'a ScalarFunction, point: &ReferenceFlowPoint) -> &'a ReferenceOrigin {
    let origins = point.possible_origins.as_ref().expect("computed fact");
    assert_eq!(origins.len(), 1, "{point:?}");
    let id = *origins.iter().next().expect("one fact");
    &function.reference_cfg.facts[id.0]
}

#[test]
fn reference_cfg_loop_and_multi_output_call_have_source_points_in_both_validators() {
    let text = "%%start\n(u8, bool)(bool) pair = fn(flag) { u8 value = 1; value, flag };\n*u8(*u8, bool) loop_ref = fn(input, flag) { *u8 result = input; while (flag) { result = null; } result };\nbool(bool) call_pair = fn(flag) { u8 magnitude, bool valid = pair(flag); magnitude; valid };\nbool(bool) assigned = fn(flag) { u8 magnitude = 0; bool valid = false; magnitude, valid = pair(flag); magnitude; valid };\n(*u8, bool)(bool) nullable = fn(flag) { null, flag };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, source) in [
        (&single.program.items, &single.program.source),
        (
            &project.project.modules[0].items,
            &project.project.modules[0].source,
        ),
    ] {
        let functions = items
            .iter()
            .filter_map(|item| match item {
                ScalarItem::Function(function) => Some(function),
                _ => None,
            })
            .collect::<Vec<_>>();
        let loop_cfg = &functions[1].reference_cfg;
        let head = loop_cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::LoopTest { .. }))
            .expect("loop head");
        let test = loop_cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::Branch { .. }))
            .expect("loop test branch");
        assert_eq!(head.source_span.source, *source);
        assert_eq!(
            head.source_span.range,
            ByteSpan::new(
                text.find("while (flag)").unwrap() as u32 + 7,
                text.find("while (flag)").unwrap() as u32 + 11
            )
        );
        assert_eq!(test.successors.len(), 2);
        assert!(loop_cfg
            .points
            .iter()
            .any(|point| point.id.0 > test.id.0 && point.successors.contains(&head.id)));
        assert!(loop_cfg
            .points
            .iter()
            .any(|point| matches!(point.kind, CfgPointKind::Join)
                && test.successors.contains(&point.id)));
        assert_eq!(
            loop_cfg
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::Return))
                .count(),
            1
        );
        let call_cfg = &functions[2].reference_cfg;
        let valid = text.find("valid = pair").unwrap() as u32;
        assert!(call_cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Bind { binding } if binding.declaration_span == ByteSpan::new(valid, valid + 5) && binding.source == *source)));
        assert!(call_cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Read { binding } if binding.declaration_span.start == valid && point.source_span.range.start > valid)));
        assert!(call_cfg
            .points
            .iter()
            .any(|point| matches!(&point.kind, CfgPointKind::Call { output: 1, .. })));
        let assignment_cfg = &functions[3].reference_cfg;
        let assignment_valid = text.rfind("valid = pair(flag); magnitude").unwrap() as u32;
        assert!(assignment_cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Assign { target, .. } if target.declaration_span.start < assignment_valid && point.source_span.range.start == assignment_valid)));
        assert!(assignment_cfg
            .points
            .iter()
            .any(|point| matches!(&point.kind, CfgPointKind::Call { output: 1, .. })));
        let nullable = &functions[4].reference_cfg;
        let null_start = text.rfind("null, flag").unwrap() as u32;
        let return_point = nullable
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::Return))
            .expect("one return");
        assert_eq!(
            return_point.source_span.range,
            ByteSpan::new(null_start, null_start + 10)
        );
        assert_eq!(
            nullable
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::Return))
                .count(),
            1
        );
        assert!(return_point.possible_origins.as_ref().is_some_and(|origins| origins.iter().any(|id| matches!(nullable.facts[id.0], ReferenceOrigin::Null { span } if span == ByteSpan::new(null_start, null_start + 4)))));
        let output_facts = nullable
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. }))
            .collect::<Vec<_>>();
        assert_eq!(output_facts.len(), 2);
        assert!(matches!(
            output_facts[0].kind,
            CfgPointKind::ReturnOutput { output: 0, .. }
        ));
        assert!(
            matches!(sole_fact(functions[4], output_facts[0]), ReferenceOrigin::Null { span } if *span == ByteSpan::new(null_start, null_start + 4))
        );
        assert!(matches!(
            output_facts[1].kind,
            CfgPointKind::ReturnOutput { output: 1, .. }
        ));
        assert!(output_facts[1].possible_origins.is_none());
    }
}

#[test]
fn returned_checked_aggregate_outputs_keep_distinct_owner_facts_in_both_validators() {
    let text = "%%start\nstruct copy Point { u8 value; }\n(*Point, *(u8[2]), *Point)(*Point) outputs = fn(borrowed) { Point sample = { .value = 1; }; u8[2] bytes = [2, 3]; &sample, &bytes, borrowed };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (diagnostics, items) in [
        (&single.diagnostics, &single.program.items),
        (&project.diagnostics, &project.project.modules[0].items),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("expected function");
        };
        let outputs = function
            .reference_cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. }))
            .collect::<Vec<_>>();
        assert_eq!(outputs.len(), 3);
        for (index, point) in outputs.iter().enumerate() {
            assert!(
                matches!(point.kind, CfgPointKind::ReturnOutput { output, .. } if output == index)
            );
        }
        for point in &outputs[..2] {
            assert!(matches!(
                sole_fact(function, point),
                ReferenceOrigin::Fresh { .. }
            ));
        }
        assert_ne!(outputs[0].possible_origins, outputs[1].possible_origins);
        assert!(matches!(
            sole_fact(function, outputs[2]),
            ReferenceOrigin::BorrowedFrom { parameter: 0, .. }
        ));
    }
}

#[test]
fn returned_checked_aggregate_branch_preserves_fresh_and_null_facts() {
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point(bool) choose = fn(flag) { Point sample = { .value = 1; }; if (flag) { &sample } else { null } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (diagnostics, items) in [
        (&single.diagnostics, &single.program.items),
        (&project.diagnostics, &project.project.modules[0].items),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let returned = function
            .reference_cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::ReturnOutput { output: 0, .. }))
            .expect("return output");
        let facts = returned.possible_origins.as_ref().expect("branch facts");
        assert_eq!(facts.len(), 2);
        assert!(facts.iter().any(|id| matches!(
            function.reference_cfg.facts[id.0],
            ReferenceOrigin::Fresh { .. }
        )));
        assert!(facts.iter().any(|id| matches!(
            function.reference_cfg.facts[id.0],
            ReferenceOrigin::Null { .. }
        )));
    }
}

#[test]
fn checked_aggregate_call_return_preserves_borrowed_owner_in_both_validators() {
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point(*Point) forward = fn(input) { input };\nunit() caller = fn { Point sample = { .value = 3; }; *Point result = forward(&sample); Point value = *result; value.value; };\n%%end";
    let (single, project) = both_reference_functions(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn forwarded_borrowed_aggregate_rejects_owner_move_at_caller_with_origin_label() {
    let text = "%%start\nstruct Payload { u8 value; }\n*Payload(*Payload) forward = fn(input) { input };\nunit() caller = fn { Payload sample = { .value = 3; }; *Payload retained = forward(&sample); Payload moved = sample; retained; moved; };\n%%end";
    let (single, project) = both_reference_functions(text);
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        let conflict = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start
                        == text.find("= sample; retained").unwrap() as u32 + 2
            })
            .unwrap_or_else(|| panic!("{diagnostics:?}"));
        assert!(conflict.labels.iter().any(|label| {
            label.kind == super::super::DiagnosticLabelKind::Secondary
                && label.span.range.start == text.find("&sample").unwrap() as u32
        }));
    }
}

#[test]
fn checked_return_rejects_escaped_moved_local_at_output_and_labels_address() {
    let text = "%%start\nstruct Payload { u8 value; }\n*Payload() escape = fn { Payload local = { .value = 1; }; *Payload held = &local; Payload moved = local; moved; held };\n%%end";
    let (single, project) = both_reference_functions(text);
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        let escaped = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start
                        == text.rfind("held };").unwrap() as u32
            })
            .unwrap_or_else(|| panic!("{diagnostics:?}"));
        assert!(escaped.labels.iter().any(|label| {
            label.kind == super::super::DiagnosticLabelKind::Secondary
                && label.span.range.start == text.find("&local").unwrap() as u32
        }));
    }
}

#[test]
fn forwarded_fresh_struct_and_array_checked_returns_validate_in_both_pipelines() {
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point() make_point = fn { Point sample = { .value = 3; }; &sample };\n*Point() forward_point = fn { make_point() };\n*(u8[2])() make_array = fn { u8[2] bytes = [4, 5]; &bytes };\n*(u8[2])() forward_array = fn { make_array() };\nunit() caller = fn { *Point result = forward_point(); Point whole = *result; u8 field = (*result).value; *(u8[2]) values = forward_array(); u8[2] copied = *values; whole.value; field; copied[0]; };\n%%end";
    let (single, project) = both_reference_functions(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn cross_module_checked_aggregate_returns_keep_declaring_source_and_concrete_owner() {
    let library_source = module_source("src/library.w");
    let library = module_from_text(
            library_source.clone(),
            "%%start\nstruct copy Point { u8 value; }\n*Point() make_point = fn { Point sample = { .value = 7; }; &sample };\n*(u8[2])() make_array = fn { u8[2] bytes = [4, 5]; &bytes };\n%%end",
        );
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nlib = namespace app \"src/library.w\";\n*lib.Point() forward_point = fn { lib.make_point() };\n*(u8[2])() forward_array = fn { lib.make_array() };\nunit() caller = fn { *lib.Point result = forward_point(); lib.Point whole = *result; u8 field = (*result).value; *(u8[2]) values = forward_array(); u8[2] copied = *values; whole.value; field; copied[0]; };\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace"),
    };
    let result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                main,
                vec![ScalarNamespaceBinding {
                    binding: "lib".to_owned(),
                    target: library_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::from_program(library, Vec::new()),
        ],
        vec![main_source, library_source.clone()],
    ));
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Function(forward) = &result.project.modules[0].items[1] else {
        panic!("forward")
    };
    let returned = forward
        .reference_cfg
        .points
        .iter()
        .find(|point| matches!(point.kind, CfgPointKind::ReturnOutput { output: 0, .. }))
        .expect("return output");
    assert!(
        matches!(sole_fact(forward, returned), ReferenceOrigin::Fresh { allocation, .. } if allocation.source == library_source)
    );
}

#[test]
fn distinct_fresh_calls_and_forwarded_null_keep_independent_output_facts() {
    let text = "%%start\nstruct copy Point { u8 value; }\n*Point() make_point = fn { Point sample = { .value = 4; }; &sample };\n*Point() empty = fn { null };\n*Point() forward_empty = fn { empty() };\nunit() caller = fn { *Point first = make_point(); *Point second = make_point(); *Point vacant = forward_empty(); Point left = *first; Point right = *second; left.value; right.value; vacant; };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let functions = items
            .iter()
            .filter_map(|item| match item {
                ScalarItem::Function(function) => Some(function),
                _ => None,
            })
            .collect::<Vec<_>>();
        let calls = functions[3]
            .reference_cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Call { output: 0, .. }))
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 3);
        let ReferenceOrigin::Fresh { loan: first, .. } = sole_fact(functions[3], calls[0]) else {
            panic!("first fresh")
        };
        let ReferenceOrigin::Fresh { loan: second, .. } = sole_fact(functions[3], calls[1]) else {
            panic!("second fresh")
        };
        assert_ne!(first.origin_place.binding, second.origin_place.binding);
        assert!(matches!(
            sole_fact(functions[3], calls[2]),
            ReferenceOrigin::Null { .. }
        ));
    }
}

#[test]
fn two_fresh_outputs_from_one_call_have_distinct_owner_places() {
    let text = "%%start\nstruct copy Point { u8 value; }\n(*Point, *Point)() pair = fn { Point first = { .value = 1; }; Point second = { .value = 2; }; &first, &second };\nunit() caller = fn { *Point left, *Point right = pair(); Point a = *left; Point b = *right; a.value; b.value; };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(caller) = &items[1] else {
            panic!("caller")
        };
        let facts = caller
            .reference_cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
            .map(|point| sole_fact(caller, point))
            .collect::<Vec<_>>();
        let [ReferenceOrigin::Fresh { loan: left, .. }, ReferenceOrigin::Fresh { loan: right, .. }] =
            facts.as_slice()
        else {
            panic!("fresh pair: {facts:?}")
        };
        assert_ne!(left.origin_place, right.origin_place);
        assert!(!reference_places_overlap(
            &left.origin_place,
            &right.origin_place,
        ));
    }
}

#[test]
fn forwarded_move_only_aggregate_checked_result_is_consumed_once() {
    let text = "%%start\nstruct Payload { u8 value; }\n*!Payload() make_payload = fn { Payload sample = { .value = 4; }; &!sample };\n*!Payload() forward_payload = fn { make_payload() };\nunit() caller = fn { *!Payload result = forward_payload(); u8 field = (*result).value; Payload taken = *result; field; taken.value; };\n%%end";
    let (single, project) = both_reference_functions(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn forwarded_move_only_fixed_array_checked_result_is_consumed_once() {
    let text = "%%start\nstruct Payload { u8 value; }\n*!(Payload[2])() make_items = fn { Payload[2] items = [{ .value = 1; }, { .value = 2; }]; &!items };\n*!(Payload[2])() forward_items = fn { make_items() };\nunit() caller = fn { *!(Payload[2]) result = forward_items(); Payload[2] taken = *result; taken; };\n%%end";
    let (single, project) = both_reference_functions(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn checked_aggregate_generic_and_overload_outputs_substitute_argument_owner() {
    let text = "%%start\nstruct copy Point { u8 value; }\ngeneric T;\n*T(*T) generic_forward = fn(input) { input };\nforward = overload { *Point(*Point) => fn(input) { generic_forward<Point>(input) }; *(u8[2])(*(u8[2])) => fn(input) { input }; };\nunit() caller = fn { Point sample = { .value = 8; }; u8[2] bytes = [1, 2]; *Point point_result = forward(&sample); *(u8[2]) array_result = forward(&bytes); Point whole = *point_result; u8[2] copied = *array_result; whole.value; copied[0]; };\n%%end";
    let (single, project) = both_reference_functions(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn returned_reborrow_of_field_preserves_disjoint_caller_place() {
    let text = "%%start\nstruct Payload { u8 value; }\nstruct Pair { Payload left; Payload right; }\n*Payload(*Payload) select_left = fn(input) { input };\nunit() caller = fn { Pair bundle = { .left = { .value = 1; }; .right = { .value = 2; }; }; *Payload left = select_left(&bundle.left); Payload right = bundle.right; left; right; };\n%%end";
    let (single, project) = both_reference_functions(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn loop_reference_facts_converge_across_zero_and_repeated_iterations_in_both_validators() {
    let text = "%%start\nunit(bool) repeat = fn(flag) { u8 first = 1; u8 second = 2; *u8 alias = &first; while (flag) { *u8 local = &second; local; alias = &second; alias = &first; } alias; };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("repeat function")
        };
        let read = function
                .reference_cfg
                .points
                .iter()
                .find(|point| {
                    matches!(&point.kind, CfgPointKind::Read { binding } if binding.declaration_span.start == text.find("alias = &first").unwrap() as u32)
                        && point.source_span.range.start == text.rfind("alias;").unwrap() as u32
                })
                .expect("exit read");
        assert_eq!(read.possible_origins.as_ref().map(BTreeSet::len), Some(2));
    }
}

#[test]
fn loop_carried_loan_conflicts_with_owner_move_after_exit_in_both_validators() {
    let text = "%%start\nstruct Payload { u8 value; }\nunit(bool) consume = fn(flag) { Payload item = { .value = 1; }; *Payload retained = &item; while (flag) { retained; } Payload taken = item; retained; taken.value; };\n%%end";
    let (single, project) = both_reference_functions(text);
    let moved = text.find("taken = item").unwrap() as u32 + 8;
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start == moved),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn loop_local_loan_ends_before_owner_move_in_both_validators() {
    let text = "%%start\nstruct Payload { u8 value; }\nunit(bool) consume = fn(flag) { Payload item = { .value = 1; }; while (flag) { *Payload local = &item; local; } Payload taken = item; taken.value; };\n%%end";
    let (single, project) = both_reference_functions(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn loop_aggregate_copy_and_move_preserve_availability_in_both_validators() {
    let copy = "%%start\nstruct copy Point { u8 value; }\nunit(bool) repeat = fn(flag) { Point item = { .value = 1; }; *Point reader = &item; while (flag) { Point seen = *reader; seen.value; } item.value; };\n%%end";
    let (single, project) = both_reference_functions(copy);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

    let moved = "%%start\nstruct Payload { u8 value; }\nunit(bool) repeat = fn(flag) { Payload item = { .value = 1; }; while (flag) { Payload taken = item; taken.value; } item.value; };\n%%end";
    let (single, project) = both_reference_functions(moved);
    let read = moved.rfind("item.value").unwrap() as u32;
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start == read),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn reference_cfg_distinguishes_core_release_from_user_free_and_records_runtime_array_moves() {
    let text = "%%start\nunit(*?u8) free = fn(pointer) { pointer; };\nunit() release = fn { unsafe { *?u8 storage = core.alloc(1, 1); free(storage); core.free(storage); } };\nu8[](u8[]) relay = fn(values) { u8[] next = values; next };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics, source) in [
        (
            &single.program.items,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let functions = items
            .iter()
            .filter_map(|item| match item {
                ScalarItem::Function(function) => Some(function),
                _ => None,
            })
            .collect::<Vec<_>>();
        let release_cfg = &functions[1].reference_cfg;
        let storage = text.find("storage =").unwrap() as u32;
        let core = text.find("core.free(storage)").unwrap() as u32;
        let user = text.find("free(storage)").unwrap() as u32;
        let releases = release_cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Release { .. }))
            .collect::<Vec<_>>();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].source_span.source, *source);
        assert_eq!(
            releases[0].source_span.range,
            ByteSpan::new(core, core + 18)
        );
        assert!(
            matches!(&releases[0].kind, CfgPointKind::Release { target: ReleaseTarget::Raw { binding } } if binding.declaration_span == ByteSpan::new(storage, storage + 7) && binding.source == *source)
        );
        assert!(release_cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Call { expression: ScalarExpression::Call { receiver: None, name, .. }, .. } if name == "free") && point.source_span.range.start == user));
        let move_cfg = &functions[2].reference_cfg;
        let parameter = text.find("fn(values)").unwrap() as u32 + 3;
        let moved = text.find("next = values").unwrap() as u32 + 7;
        assert!(move_cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Move { place, .. } if place.binding.declaration_span == ByteSpan::new(parameter, parameter + 6) && place.binding.source == *source) && point.source_span.range == ByteSpan::new(moved, moved + 6)));
    }
}

#[test]
fn reference_cfg_call_targets_distinguish_core_free_from_local_free_in_both_validators() {
    let text = "%%start\nunit((unit(*?u8))) release = fn(free) { unsafe { *?u8 storage = core.alloc(1, 1); free(storage); core.free(storage); } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics, source) in [
        (
            &single.program.items,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("source function")
        };
        let calls = function
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Call {
                    target, output: 0, ..
                } => Some((point, target)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 3);
        let alloc_start = text.find("core.alloc(1, 1)").unwrap() as u32;
        let local_start = text.find("free(storage)").unwrap() as u32;
        let core_start = text.find("core.free(storage)").unwrap() as u32;
        assert_eq!(
            calls[0].0.source_span,
            SourceSpan::new(source.clone(), ByteSpan::new(alloc_start, alloc_start + 16))
        );
        assert_eq!(
            calls[0].1,
            &ResolvedCallTarget::Core(CoreOperationId::Alloc)
        );
        let ResolvedCallTarget::LocalCallable(local) = calls[1].1 else {
            panic!(
                "local free call must use resolved lexical binding: {:?}",
                calls[1].1
            )
        };
        let parameter_start = text.find("fn(free)").unwrap() as u32 + 3;
        assert_eq!(
            local.declaration_span,
            ByteSpan::new(parameter_start, parameter_start + 4)
        );
        assert_eq!(local.function_span, function.span);
        assert_eq!(local.source, *source);
        assert_eq!(
            calls[1].0.source_span.range,
            ByteSpan::new(local_start, local_start + 13)
        );
        assert_eq!(calls[2].1, &ResolvedCallTarget::Core(CoreOperationId::Free));
        assert_eq!(
            calls[2].0.source_span.range,
            ByteSpan::new(core_start, core_start + 18)
        );
        let releases = function
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Release {
                    target: ReleaseTarget::Raw { binding },
                } => Some((point, binding)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(releases.len(), 1);
        let storage_start = text.find("storage =").unwrap() as u32;
        assert_eq!(
            releases[0].1.declaration_span,
            ByteSpan::new(storage_start, storage_start + 7)
        );
        assert_eq!(releases[0].1.source, *source);
        assert_eq!(
            releases[0].0.source_span.range,
            calls[2].0.source_span.range
        );
    }
}

#[test]
fn checked_core_free_records_typed_owner_and_expires_derived_loans_in_both_validators() {
    let text = "%%start\nunit(u64) release = fn(count) { u8[count] bytes; *u8 reader = &bytes[0]; reader; unsafe { core.free(&bytes); } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics, source) in [
        (
            &single.program.items,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("release")
        };
        let release = function
            .reference_cfg
            .points
            .iter()
            .find(|point| {
                matches!(
                    &point.kind,
                    CfgPointKind::Release {
                        target: ReleaseTarget::Checked { .. }
                    }
                )
            })
            .expect("checked release");
        let CfgPointKind::Release {
            target:
                ReleaseTarget::Checked {
                    address_point,
                    candidates,
                    address_span,
                },
        } = &release.kind
        else {
            unreachable!()
        };
        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0];
        let bytes_start = text.find("bytes;").unwrap() as u32;
        assert_eq!(
            candidate.place.binding.declaration_span,
            ByteSpan::new(bytes_start, bytes_start + 5)
        );
        assert_eq!(candidate.place.source, *source);
        assert!(candidate.place.projections.is_empty());
        assert_eq!(candidate.loan.origin_place, candidate.place);
        assert!(function.reference_cfg.points[address_point.0]
            .possible_origins
            .as_ref()
            .unwrap()
            .contains(&candidate.origin));
        let address_start = text.find("&bytes);").unwrap() as u32;
        assert_eq!(
            address_span,
            &SourceSpan::new(
                source.clone(),
                ByteSpan::new(address_start, address_start + 6)
            )
        );
    }
    let bad = "%%start\nunit(u64) release = fn(count) { u8[count] bytes; *u8 reader = &bytes[0]; unsafe { core.free(&bytes); }; reader; };\n%%end";
    let (single, project) = both_reference_functions(bad);
    let read_start = bad.rfind("reader;").unwrap() as u32;
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range
                        == ByteSpan::new(read_start, read_start + 6)),
            "{diagnostics:?}"
        );
    }
    let owner_use = "%%start\nunit(u64) release = fn(count) { u8[count] bytes; unsafe { core.free(&bytes); }; bytes[0]; };\n%%end";
    let (single, project) = both_reference_functions(owner_use);
    let use_start = owner_use.rfind("bytes[0]").unwrap() as u32;
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start == use_start),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn checked_core_free_retains_all_joined_owner_candidates_in_both_validators() {
    let text = "%%start\nunit(u64, bool) release = fn(count, flag) { u8[count] first; u8[count] second; *u8 alias = &first[0]; if (flag) { alias = &second[0]; }; unsafe { core.free(alias); } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics, source) in [
        (
            &single.program.items,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("release")
        };
        let point = function
            .reference_cfg
            .points
            .iter()
            .find(|point| {
                matches!(
                    &point.kind,
                    CfgPointKind::Release {
                        target: ReleaseTarget::Checked { .. }
                    }
                )
            })
            .expect("checked release");
        let CfgPointKind::Release {
            target:
                ReleaseTarget::Checked {
                    candidates,
                    address_point,
                    address_span,
                },
        } = &point.kind
        else {
            unreachable!()
        };
        assert_eq!(candidates.len(), 2);
        let read = &function.reference_cfg.points[address_point.0];
        assert_eq!(read.possible_origins.as_ref().map(BTreeSet::len), Some(2));
        let alias_start = text.rfind("alias);").unwrap() as u32;
        assert_eq!(
            address_span,
            &SourceSpan::new(source.clone(), ByteSpan::new(alias_start, alias_start + 5))
        );
        let declarations = [
            text.find("first;").unwrap() as u32,
            text.find("second;").unwrap() as u32,
        ];
        for candidate in candidates {
            assert!(read
                .possible_origins
                .as_ref()
                .unwrap()
                .contains(&candidate.origin));
            assert_eq!(candidate.place.source, *source);
            assert_eq!(candidate.loan.origin_place, candidate.place);
            assert_eq!(candidate.place.projections.len(), 1);
            assert!(declarations.contains(&candidate.place.binding.declaration_span.start));
        }
        assert_ne!(candidates[0].place.binding, candidates[1].place.binding);
    }
}

#[test]
fn joined_checked_release_preserves_untouched_owner_on_each_branch() {
    let text = "%%start\nunit(u64, bool) release = fn(count, flag) { u8[count] first; u8[count] second; *u8 alias = &first[0]; if (flag) { alias = &second[0]; unsafe { core.free(alias); }; first[0]; } else { unsafe { core.free(alias); }; second[0]; }; } ;\n%%end";
    let (single, project) = both_reference_functions(text);
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}

#[test]
fn joined_checked_release_rejects_use_of_selected_owner_on_each_branch() {
    let text = "%%start\nunit(u64, bool) release = fn(count, flag) { u8[count] first; u8[count] second; *u8 alias = &first[0]; if (flag) { alias = &second[0]; unsafe { core.free(alias); }; *u8 used = &second[0]; used; } else { unsafe { core.free(alias); }; *u8 used = &first[0]; used; }; } ;\n%%end";
    let (single, project) = both_reference_functions(text);
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        for owner in ["&second[0]; used", "&first[0]; used"] {
            let start = text.find(owner).unwrap() as u32;
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "B0003"
                        && diagnostic.labels[0].span.range.start == start),
                "{diagnostics:?}"
            );
        }
    }
}

#[test]
fn joined_checked_release_reports_second_release_for_each_possible_owner() {
    let text = "%%start\nunit(u64, bool) release = fn(count, flag) { u8[count] first; u8[count] second; *u8 alias = &first[0]; if (flag) { alias = &second[0]; }; unsafe { core.free(alias); core.free(alias); }; } ;\n%%end";
    let (single, project) = both_reference_functions(text);
    let second = text.rfind("alias);").unwrap() as u32;
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start == second),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn joined_checked_release_preserves_correlated_untouched_alias() {
    let text = "%%start\nunit(u64, bool) release = fn(count, flag) { u8[count] first; u8[count] second; *u8 selected = &first[0]; *u8 untouched = &second[0]; if (flag) { selected, untouched = untouched, selected; }; unsafe { core.free(selected); }; untouched; } ;\n%%end";
    let (single, project) = both_reference_functions(text);
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}

#[test]
fn checked_core_free_validates_unsafe_arity_types_and_owner_reuse_in_both_validators() {
    for (text, code, marker) in [
            (
                "%%start\nunit(u64) release = fn(count) { u8[count] bytes; core.free(&bytes); };\n%%end",
                "B0012",
                "core.free(&bytes)",
            ),
            (
                "%%start\nunit(u64) release = fn(count) { u8[count] bytes; unsafe { core.free(); }; };\n%%end",
                "B0004",
                "core.free()",
            ),
            (
                "%%start\nunit(u64) release = fn(count) { u8[count] bytes; unsafe { core.free(&bytes, &bytes); }; };\n%%end",
                "B0004",
                "core.free(&bytes, &bytes)",
            ),
            (
                "%%start\nunit(u64) release = fn(count) { u8[count] bytes; unsafe { core.free(count); }; };\n%%end",
                "B0003",
                "count);",
            ),
            (
                "%%start\nunit(u8) release = fn(value) { unsafe { core.free(&value); }; };\n%%end",
                "B0003",
                "&value",
            ),
            (
                "%%start\nunit(u64) release = fn(count) { u8[count] bytes; unsafe { core.free(&bytes); core.free(&bytes); }; };\n%%end",
                "B0003",
                "core.free(&bytes);",
            ),
        ] {
            let (single, project) = both_reference_functions(text);
            for diagnostics in [&single.diagnostics, &project.diagnostics] {
                assert!(
                    diagnostics.iter().any(|diagnostic| diagnostic.code == code
                        && diagnostic.labels[0].span.range.start
                            >= text.find(marker).unwrap() as u32),
                    "{code}: {diagnostics:?}"
                );
            }
        }
    let duplicate = "%%start\nunit(u64) release = fn(count) { u8[count] bytes; unsafe { core.free(&bytes); core.free(&bytes); }; };\n%%end";
    let (single, project) = both_reference_functions(duplicate);
    let second_address = duplicate.rfind("&bytes").unwrap() as u32;
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start == second_address),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn callable_struct_field_free_has_source_identity_in_both_validators() {
    let text = "%%start\nstruct Pool { unit(*?u8) free; }\nunit(*?u8) release = fn(storage) { storage; };\nunit((unit(*?u8))) invoke = fn(free) { Pool heap = { .free = release; }; unsafe { *?u8 storage = core.alloc(1, 1); free(storage); heap.free(storage); core.free(storage); } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, structs, diagnostics, source) in [
        (
            &single.program.items,
            &single.program.structs,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.project.modules[0].structs,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[1] else {
            panic!("invoke")
        };
        let field = &structs[0].fields[0];
        let field_start = text.find("free; }").unwrap() as u32;
        assert_eq!(field.name_span, ByteSpan::new(field_start, field_start + 4));
        let heap_start = text.find("heap =").unwrap() as u32;
        let local_start = text.find("free(storage);").unwrap() as u32;
        let call_start = text.find("heap.free(storage)").unwrap() as u32;
        let core_start = text.find("core.free(storage)").unwrap() as u32;
        let calls = function
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Call {
                    output: 0, target, ..
                } => Some((point, target)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 4);
        assert_eq!(
            calls[0].1,
            &ResolvedCallTarget::Core(CoreOperationId::Alloc)
        );
        let ResolvedCallTarget::LocalCallable(local) = calls[1].1 else {
            panic!("local free")
        };
        let parameter_start = text.find("fn(free)").unwrap() as u32 + 3;
        assert_eq!(
            local.declaration_span,
            ByteSpan::new(parameter_start, parameter_start + 4)
        );
        assert_eq!(local.source, *source);
        assert_eq!(calls[1].0.source_span.range.start, local_start);
        let ResolvedCallTarget::CallableField {
            receiver_place,
            field: field_id,
        } = calls[2].1
        else {
            panic!("typed field target: {:?}", calls[2].1)
        };
        assert_eq!(field_id, &field.id);
        assert_eq!(field_id.structure, structs[0].id);
        assert_eq!(receiver_place.source, *source);
        assert_eq!(receiver_place.function_span, function.span);
        assert_eq!(
            receiver_place.declaration_span,
            ByteSpan::new(heap_start, heap_start + 4)
        );
        assert_eq!(
            receiver_place.binding.declaration_span,
            receiver_place.declaration_span
        );
        assert_eq!(receiver_place.binding.source, *source);
        assert_eq!(receiver_place.binding.block_span, function.body.span);
        assert!(receiver_place.projections.is_empty());
        assert_eq!(
            receiver_place.place,
            ScalarPlace::Name {
                name: "heap".to_owned(),
                span: ByteSpan::new(call_start, call_start + 4)
            }
        );
        assert_eq!(
            calls[2].0.source_span,
            SourceSpan::new(source.clone(), ByteSpan::new(call_start, call_start + 18))
        );
        assert_eq!(calls[3].1, &ResolvedCallTarget::Core(CoreOperationId::Free));
        assert_eq!(
            calls[3].0.source_span.range,
            ByteSpan::new(core_start, core_start + 18)
        );
        assert_eq!(
            function
                .reference_cfg
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::Release { .. }))
                .count(),
            1
        );
        assert!(function.reference_cfg.points.iter().any(|point| matches!(
            &point.kind,
            CfgPointKind::Release { .. }
        ) && point
            .source_span
            .range
            .start
            == core_start));
    }
}

#[test]
fn callable_struct_field_reports_unknown_noncallable_and_argument_errors_in_both_validators() {
    for (text, code, marker) in [
            (
                "%%start\nstruct Pool { unit(*?u8) free; }\nunit(*?u8) release = fn(storage) { storage; };\nunit() invoke = fn { Pool heap = { .free = release; }; heap.missing(); };\n%%end",
                "B0001",
                "missing()",
            ),
            (
                "%%start\nstruct Pool { u8 free; }\nunit() invoke = fn { Pool heap = { .free = 1; }; heap.free(); };\n%%end",
                "B0003",
                "free()",
            ),
            (
                "%%start\nstruct Pool { unit(*?u8) free; }\nunit(*?u8) release = fn(storage) { storage; };\nunit() invoke = fn { Pool heap = { .free = release; }; heap.free(); };\n%%end",
                "B0004",
                "heap.free()",
            ),
            (
                "%%start\nstruct Pool { unit(*?u8) free; }\nunit(*?u8) release = fn(storage) { storage; };\nunit() invoke = fn { Pool heap = { .free = release; }; heap.free(1); };\n%%end",
                "B0003",
                "1);",
            ),
        ] {
            let (single, project) = both_reference_functions(text);
            let start = text.rfind(marker).unwrap() as u32;
            for diagnostics in [&single.diagnostics, &project.diagnostics] {
                assert!(
                    diagnostics.iter().any(|diagnostic| diagnostic.code == code
                        && diagnostic.labels[0].span.range.start == start),
                    "{code} {marker}: {diagnostics:?}"
                );
            }
        }
}

#[test]
fn reference_cfg_qualified_overloads_keep_distinct_module_sources_and_arms() {
    let left_source = module_source("src/left.w");
    let right_source = module_source("src/right.w");
    let main_source = module_source("src/main.w");
    let left_text = "%%start\npick = overload { i32(i32) => fn(value) { value }; i32(i64) => fn(value) { 2 }; };\n%%end";
    let right_text = "%%start\npick = overload { i32(i32) => fn(value) { value }; i32(i64) => fn(value) { 3 }; };\n%%end";
    let main_text = "%%start\nleft = namespace app \"src/left.w\";\nright = namespace app \"src/right.w\";\ni32(i32, i64) choose = fn(narrow, wide) { i32 a = left.pick(narrow); i32 b = right.pick(wide); a + b };\n%%end";
    let left = module_from_text(left_source.clone(), left_text);
    let right = module_from_text(right_source.clone(), right_text);
    let main = module_from_text(main_source.clone(), main_text);
    let namespace_bindings = main
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Namespace(namespace) => Some(ScalarNamespaceBinding {
                binding: namespace.binding.clone(),
                target: if namespace.binding == "left" {
                    left_source.clone()
                } else {
                    right_source.clone()
                },
                span: namespace.span,
            }),
            _ => None,
        })
        .collect();
    let input = ScalarProject::new(
        vec![
            ScalarModule::new(main_source.clone(), main.items, namespace_bindings),
            ScalarModule::new(left_source.clone(), left.items, Vec::new()),
            ScalarModule::new(right_source.clone(), right.items, Vec::new()),
        ],
        vec![
            main_source.clone(),
            left_source.clone(),
            right_source.clone(),
        ],
    );
    let result = validate_scalar_project(input);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Function(function) = &result.project.modules[0].items[2] else {
        panic!("choose function")
    };
    let calls = function
        .reference_cfg
        .points
        .iter()
        .filter_map(|point| match &point.kind {
            CfgPointKind::Call {
                target:
                    ResolvedCallTarget::ModuleCallable {
                        source,
                        declaration_span,
                        concrete,
                    },
                output: 0,
                ..
            } => Some((point, source, declaration_span, concrete)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    for (index, (point, source, declaration_span, concrete)) in calls.iter().enumerate() {
        let (expected_source, text) = if index == 0 {
            (&left_source, left_text)
        } else {
            (&right_source, right_text)
        };
        assert_eq!(*source, expected_source);
        assert_eq!(
            **declaration_span,
            ByteSpan::new(
                text.find("pick =").unwrap() as u32,
                text.find("pick =").unwrap() as u32 + 4
            )
        );
        assert_eq!(point.source_span.source, main_source);
        let call = if index == 0 {
            "left.pick(narrow)"
        } else {
            "right.pick(wide)"
        };
        let start = main_text.find(call).unwrap() as u32;
        assert_eq!(
            point.source_span.range,
            ByteSpan::new(start, start + call.len() as u32)
        );
        let ConcreteCallSelection::Overload(selection) = concrete else {
            panic!("concrete overload arm")
        };
        assert_eq!(selection.arm_index, index);
    }
}

#[test]
fn core_invalidate_and_rebind_record_typed_places_and_inputs_in_both_validators() {
    let text = "%%start\nunit(u64, *?u8) update = fn(count, raw) { u8[count] bytes; *u8 first = &bytes[0]; first; unsafe { core.invalidate(&bytes); core.rebind(&bytes, raw, count); } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics, source) in [
        (
            &single.program.items,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("update function")
        };
        let events = function
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Invalidate {
                    address_point,
                    candidates,
                    operation,
                    address_span,
                    inputs,
                } => Some((
                    point,
                    address_point,
                    candidates,
                    operation,
                    address_span,
                    inputs,
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        let bytes_start = text.find("bytes;").unwrap() as u32;
        for (index, &(point, address_point, candidates, operation, address_span, inputs)) in
            events.iter().enumerate()
        {
            assert_eq!(candidates.len(), 1);
            let candidate = &candidates[0];
            let place = &candidate.place;
            let call = if index == 0 {
                "core.invalidate(&bytes)"
            } else {
                "core.rebind(&bytes, raw, count)"
            };
            let call_start = text.find(call).unwrap() as u32;
            assert_eq!(
                point.source_span,
                SourceSpan::new(
                    source.clone(),
                    ByteSpan::new(call_start, call_start + call.len() as u32)
                )
            );
            assert_eq!(
                address_span,
                &SourceSpan::new(
                    source.clone(),
                    ByteSpan::new(
                        call_start + if index == 0 { 16 } else { 12 },
                        call_start + if index == 0 { 22 } else { 18 }
                    )
                )
            );
            assert!(address_point.0 < point.id.0);
            assert!(function.reference_cfg.points[address_point.0]
                .possible_origins
                .as_ref()
                .is_some_and(|ids| ids.contains(&candidate.origin)));
            assert_eq!(candidate.loan.origin_place, *place);
            assert_eq!(place.source, *source);
            assert_eq!(
                place.binding.declaration_span,
                ByteSpan::new(bytes_start, bytes_start + 5)
            );
            assert_eq!(
                place.place,
                ScalarPlace::Name {
                    name: "bytes".to_owned(),
                    span: ByteSpan::new(
                        call_start + if index == 0 { 17 } else { 13 },
                        call_start + if index == 0 { 22 } else { 18 }
                    )
                }
            );
            assert!(place.projections.is_empty());
            assert_eq!(
                operation,
                &if index == 0 {
                    CoreOperationId::Invalidate
                } else {
                    CoreOperationId::Rebind
                }
            );
            let call_point = function.reference_cfg.points.iter().find(|candidate| candidate.source_span == point.source_span && matches!(&candidate.kind, CfgPointKind::Call { target: ResolvedCallTarget::Core(core), .. } if core == operation)).expect("resolved core call");
            assert!(call_point.id.0 > point.id.0);
            match inputs {
                InvalidationInputs::Invalidate => assert_eq!(index, 0),
                InvalidationInputs::Free => panic!("free is a release event"),
                InvalidationInputs::Rebind {
                    raw_address,
                    raw_span,
                    length,
                    length_span,
                } => {
                    assert_eq!(index, 1);
                    assert!(
                        matches!(raw_address, ScalarExpression::Name { name, .. } if name == "raw")
                    );
                    assert!(
                        matches!(length, ScalarExpression::Name { name, .. } if name == "count")
                    );
                    let raw_start = text.find("raw, count);").unwrap() as u32;
                    assert_eq!(
                        raw_span,
                        &SourceSpan::new(source.clone(), ByteSpan::new(raw_start, raw_start + 3))
                    );
                    assert_eq!(
                        length_span,
                        &SourceSpan::new(
                            source.clone(),
                            ByteSpan::new(raw_start + 5, raw_start + 10)
                        )
                    );
                }
            }
        }
    }
}

#[test]
fn core_invalidate_and_rebind_reject_unsafe_arity_and_argument_types_in_both_validators() {
    let cases = [
        (
            "core.invalidate(&bytes);",
            "B0012",
            "core.invalidate(&bytes)",
        ),
        (
            "core.rebind(&bytes, raw, count);",
            "B0012",
            "core.rebind(&bytes, raw, count)",
        ),
        (
            "unsafe { core.invalidate(); }",
            "B0004",
            "core.invalidate()",
        ),
        (
            "unsafe { core.rebind(&bytes, raw); }",
            "B0004",
            "core.rebind(&bytes, raw)",
        ),
        ("unsafe { core.invalidate(raw); }", "B0003", "raw);"),
        (
            "unsafe { core.rebind(&bytes, count, count); }",
            "B0003",
            "count, count);",
        ),
        (
            "unsafe { core.rebind(&bytes, raw, raw); }",
            "B0003",
            "raw); }",
        ),
    ];
    for (statement, code, fragment) in cases {
        let text = format!(
                "%%start\nunit(u64, *?u8) update = fn(count, raw) {{ u8[count] bytes; {statement} }};\n%%end"
            );
        let (single, project) = both_reference_functions(&text);
        for diagnostics in [&single.diagnostics, &project.diagnostics] {
            let start = text.rfind(fragment).unwrap() as u32;
            let end = start
                + if code == "B0003" {
                    if fragment == "count, count);" {
                        5
                    } else {
                        3
                    }
                } else {
                    fragment.len() as u32
                };
            assert!(
                diagnostics.iter().any(|diagnostic| diagnostic.code == code
                    && diagnostic.labels[0].span.range.start == start
                    && diagnostic.labels[0].span.range.end == end),
                "{statement}: {diagnostics:?}"
            );
        }
    }
}

#[test]
fn core_invalidate_preserves_resolved_index_projection_in_both_validators() {
    let text = "%%start\nunit(u64) update = fn(count) { u8[count] bytes; unsafe { core.invalidate(&bytes[0]); } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics, source) in [
        (
            &single.program.items,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let point = function
            .reference_cfg
            .points
            .iter()
            .find(|point| matches!(&point.kind, CfgPointKind::Invalidate { .. }))
            .expect("invalidation");
        let CfgPointKind::Invalidate {
            candidates,
            address_span,
            operation,
            ..
        } = &point.kind
        else {
            unreachable!()
        };
        assert_eq!(operation, &CoreOperationId::Invalidate);
        assert_eq!(candidates.len(), 1);
        let place = &candidates[0].place;
        assert_eq!(place.source, *source);
        assert_eq!(place.projections.len(), 1);
        assert!(
            matches!(&place.projections[0], ReferencePlaceProjection::Index(ScalarExpression::Integer { value, span }) if value == &BigInt::from(0) && *span == ByteSpan::new(text.find("[0]").unwrap() as u32 + 1, text.find("[0]").unwrap() as u32 + 2))
        );
        let start = text.find("&bytes[0]").unwrap() as u32;
        assert_eq!(
            address_span,
            &SourceSpan::new(source.clone(), ByteSpan::new(start, start + 9))
        );
    }
}

#[test]
fn core_invalidate_preserves_both_joined_alias_places_in_both_validators() {
    let text = "%%start\nunit(u8, u8, bool) invalidate_alias = fn(first, second, flag) { *u8 alias = &first; if (flag) { alias = &second; }; unsafe { core.invalidate(alias); } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics, source) in [
        (
            &single.program.items,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let event = function
            .reference_cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::Invalidate { .. }))
            .expect("invalidation event");
        let CfgPointKind::Invalidate {
            address_point,
            candidates,
            address_span,
            operation,
            ..
        } = &event.kind
        else {
            unreachable!()
        };
        assert_eq!(operation, &CoreOperationId::Invalidate);
        let alias_start = text.rfind("alias);").unwrap() as u32;
        assert_eq!(
            address_span,
            &SourceSpan::new(source.clone(), ByteSpan::new(alias_start, alias_start + 5))
        );
        let read = &function.reference_cfg.points[address_point.0];
        assert!(matches!(read.kind, CfgPointKind::Read { .. }));
        assert_eq!(read.source_span, *address_span);
        assert_eq!(candidates.len(), 2);
        assert_eq!(read.possible_origins.as_ref().map(BTreeSet::len), Some(2));
        let first_start = text.find("fn(first").unwrap() as u32 + 3;
        let second_start = text.find("second, flag").unwrap() as u32;
        let mut declarations = candidates
            .iter()
            .map(|candidate| {
                assert_eq!(candidate.place.source, *source);
                assert_eq!(candidate.place.binding.source, *source);
                assert_eq!(candidate.loan.origin_place, candidate.place);
                assert!(read
                    .possible_origins
                    .as_ref()
                    .unwrap()
                    .contains(&candidate.origin));
                assert!(candidate.place.projections.is_empty());
                candidate.place.binding.declaration_span
            })
            .collect::<Vec<_>>();
        declarations.sort_by_key(|span| span.start);
        assert_eq!(
            declarations,
            vec![
                ByteSpan::new(first_start, first_start + 5),
                ByteSpan::new(second_start, second_start + 6),
            ]
        );
        let creation_spans = candidates
            .iter()
            .map(|candidate| candidate.loan.creation_span)
            .collect::<Vec<_>>();
        assert!(creation_spans.contains(&ByteSpan::new(
            text.find("&first").unwrap() as u32,
            text.find("&first").unwrap() as u32 + 6
        )));
        assert!(creation_spans.contains(&ByteSpan::new(
            text.find("&second").unwrap() as u32,
            text.find("&second").unwrap() as u32 + 7
        )));
    }
}

#[test]
fn core_invalidation_and_rebind_expire_derived_reference_reads_in_both_validators() {
    for (operation, text) in [
            (
                "invalidate",
                "%%start\nunit(u64) expire = fn(count) { u8[count] bytes; *u8 old = &bytes[0]; unsafe { core.invalidate(&bytes); }; old; };\n%%end",
            ),
            (
                "rebind",
                "%%start\nunit(u64, *?u8) expire = fn(count, raw) { u8[count] bytes; *u8 old = &bytes[0]; unsafe { core.rebind(&bytes, raw, count); }; old; };\n%%end",
            ),
        ] {
            let (single, project) = both_reference_functions(text);
            let start = text.rfind("old;").unwrap() as u32;
            for diagnostics in [&single.diagnostics, &project.diagnostics] {
                assert!(
                    diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == "B0003"
                            && diagnostic.labels[0].span.range == ByteSpan::new(start, start + 3)),
                    "{operation}: {diagnostics:?}"
                );
            }
        }
}

#[test]
fn local_invalidate_callable_has_no_core_invalidation_event() {
    let text = "%%start\nunit((unit(*u8)), u8) invoke = fn(invalidate, bytes) { invalidate(&bytes); };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        assert!(!function
            .reference_cfg
            .points
            .iter()
            .any(|point| matches!(point.kind, CfgPointKind::Invalidate { .. })));
        let call = function
            .reference_cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::Call { .. }))
            .expect("local call");
        assert!(
            matches!(&call.kind, CfgPointKind::Call { target: ResolvedCallTarget::LocalCallable(binding), .. } if binding.declaration_span.start == text.find("fn(invalidate").unwrap() as u32 + 3)
        );
    }
}

#[test]
fn reference_cfg_branches_join_before_final_read_in_both_validators() {
    let text = "%%start\n*u8(*u8, *u8, bool) choose = fn(first, second, flag) { *u8 result = first; if (flag) { result = first; } else { result = second; }; result };\n*u8(*u8, bool) maybe = fn(input, flag) { *u8 result = input; if (flag) { result = null; }; result };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics, source) in [
        (
            &single.program.items,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        let functions = items
            .iter()
            .map(|item| match item {
                ScalarItem::Function(function) => function,
                _ => panic!("expected source function: {item:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(functions.len(), 2, "{diagnostics:?}");
        for function in functions {
            let cfg = &function.reference_cfg;
            let branch = cfg
                .points
                .iter()
                .find(|point| matches!(point.kind, CfgPointKind::Branch { .. }))
                .expect("branch");
            let join = cfg
                .points
                .iter()
                .find(|point| matches!(point.kind, CfgPointKind::Join))
                .expect("join");
            let condition_start = text[function.span.start as usize..]
                .find("if (flag)")
                .unwrap() as u32
                + function.span.start
                + 4;
            assert_eq!(
                branch.source_span.range,
                ByteSpan::new(condition_start, condition_start + 4)
            );
            assert_eq!(&branch.source_span.source, source);
            assert_eq!(branch.successors.len(), 2);
            assert_ne!(branch.successors[0], branch.successors[1]);
            assert_eq!(
                cfg.points[branch.successors[0].0].kind,
                CfgPointKind::ScopeEnter
            );
            if function.name == "choose" {
                assert_eq!(
                    cfg.points[branch.successors[1].0].kind,
                    CfgPointKind::ScopeEnter
                );
            } else {
                assert_eq!(branch.successors[1], join.id);
            }
            let predecessors = cfg
                .points
                .iter()
                .filter(|point| point.successors.contains(&join.id))
                .collect::<Vec<_>>();
            assert_eq!(predecessors.len(), 2);
            assert_ne!(predecessors[0].id, predecessors[1].id);
            assert!(predecessors
                .iter()
                .any(|point| matches!(point.kind, CfgPointKind::ScopeExit)));
            assert!(predecessors.iter().all(|point| matches!(
                point.kind,
                CfgPointKind::ScopeExit | CfgPointKind::Branch { .. }
            )));
            assert_eq!(&join.source_span.source, source);
            assert_eq!(
                join.source_span.range,
                function
                    .body
                    .items
                    .iter()
                    .find_map(|item| match item {
                        ScalarBlockItem::Expression(
                            ScalarExpression::If { span, .. }
                            | ScalarExpression::UnitIf { span, .. },
                        ) => Some(*span),
                        _ => None,
                    })
                    .expect("typed conditional span")
            );
            if function.name == "choose" {
                assert_ne!(
                    cfg.points[branch.successors[0].0].scope,
                    cfg.points[branch.successors[1].0].scope
                );
            }
            let declaration = text[function.span.start as usize..]
                .find("result = ")
                .unwrap() as u32
                + function.span.start;
            let final_read = text[function.span.start as usize..function.span.end as usize]
                .rfind("result")
                .unwrap() as u32
                + function.span.start;
            let read = cfg.points.iter().find(|point| matches!(&point.kind, CfgPointKind::Read { binding } if binding.declaration_span.start == declaration) && point.id.0 > join.id.0).expect("actual final read");
            assert_eq!(join.successors, vec![read.id]);
            assert_eq!(read.possible_origins.as_ref().map(BTreeSet::len), Some(2));
            assert_eq!(
                read.source_span.range,
                ByteSpan::new(final_read, final_read + 6)
            );
            assert_eq!(&read.source_span.source, source);
            let CfgPointKind::Read { binding } = &read.kind else {
                unreachable!()
            };
            assert_eq!(&binding.source, source);
            assert_eq!(binding.function_span, function.span);
            assert_eq!(
                binding.declaration_span,
                ByteSpan::new(declaration, declaration + 6)
            );
            for (value, expected) in [("result = first;", 0), ("result = second;", 1)] {
                if function.name == "choose" {
                    let target = text[branch.source_span.range.start as usize..]
                        .find(value)
                        .unwrap() as u32
                        + branch.source_span.range.start;
                    assert!(cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Assign { target: place, .. } if &place.binding == binding) && point.source_span.range == ByteSpan::new(target, target + 6) && point.id.0 > branch.successors[expected].0 && point.id.0 < join.id.0));
                }
            }
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
        }
    }
}

#[test]
fn reference_cfg_joins_source_derived_parameter_and_null_or_distinct_fresh_borrows() {
    for (text, fresh) in [
            (
                "%%start\n*u8(*u8, bool) choose = fn(input, flag) { *u8 result = null; if (flag) { result = input; } else { result = null; }; result };\n%%end",
                false,
            ),
            (
                "%%start\n*u8(u8, u8, bool) choose = fn(first, second, flag) { *u8 result = null; if (flag) { result = &first; } else { result = &second; }; result };\n%%end",
                true,
            ),
        ] {
            let (single, project) = both_reference_functions(text);
            for (items, diagnostics, source) in [
                (
                    &single.program.items,
                    &single.diagnostics,
                    &single.program.source,
                ),
                (
                    &project.project.modules[0].items,
                    &project.diagnostics,
                    &project.project.modules[0].source,
                ),
            ] {
                assert!(diagnostics.is_empty(), "{diagnostics:?}");
                let ScalarItem::Function(function) = &items[0] else {
                    panic!("source function")
                };
                let cfg = &function.reference_cfg;
                let join = cfg
                    .points
                    .iter()
                    .find(|p| matches!(p.kind, CfgPointKind::Join))
                    .expect("join");
                let predecessors = cfg
                    .points
                    .iter()
                    .filter(|p| p.successors.contains(&join.id))
                    .collect::<Vec<_>>();
                assert_eq!(predecessors.len(), 2);
                assert_ne!(predecessors[0].id, predecessors[1].id);
                let read = cfg
                    .points
                    .iter()
                    .find(|p| p.id.0 > join.id.0 && matches!(p.kind, CfgPointKind::Read { .. }))
                    .expect("actual final read");
                assert_eq!(join.successors, vec![read.id]);
                assert_eq!(read.source_span.source, *source);
                let final_start = text.rfind("result };").unwrap() as u32;
                assert_eq!(
                    read.source_span.range,
                    ByteSpan::new(final_start, final_start + 6)
                );
                let final_origins = read
                    .possible_origins
                    .as_ref()
                    .expect("computed final origins");
                assert_eq!(final_origins.len(), 2);
                let assignments = cfg
                    .points
                    .iter()
                    .filter(|p| matches!(p.kind, CfgPointKind::Assign { .. }))
                    .collect::<Vec<_>>();
                assert_eq!(assignments.len(), 2);
                let initial = cfg.points.iter().find(|p| matches!(&p.kind, CfgPointKind::Bind { binding } if binding.declaration_span.start == text.find("result = null").unwrap() as u32)).expect("initial binding");
                let initial_ids = initial.possible_origins.as_ref().expect("initial null");
                assert_eq!(initial_ids.len(), 1);
                for assignment in &assignments {
                    let ids = assignment
                        .possible_origins
                        .as_ref()
                        .expect("edge-specific assignment");
                    assert_eq!(ids.len(), 1);
                    assert!(ids.is_disjoint(initial_ids));
                    assert!(ids.is_subset(final_origins));
                    assert_eq!(assignment.source_span.source, *source);
                }
                assert_ne!(
                    assignments[0].possible_origins,
                    assignments[1].possible_origins
                );
                let facts = final_origins
                    .iter()
                    .map(|id| &cfg.facts[id.0])
                    .collect::<Vec<_>>();
                if fresh {
                    let fresh_facts = facts
                        .iter()
                        .map(|fact| match fact {
                            ReferenceOrigin::Fresh { allocation, loan } => (allocation, loan),
                            _ => panic!("fresh branch fact: {fact:?}"),
                        })
                        .collect::<Vec<_>>();
                    assert_ne!(fresh_facts[0].0, fresh_facts[1].0);
                    assert_ne!(
                        fresh_facts[0].1.origin_place.binding,
                        fresh_facts[1].1.origin_place.binding
                    );
                    assert_ne!(
                        fresh_facts[0].1.creation_span,
                        fresh_facts[1].1.creation_span
                    );
                    for (allocation, loan) in fresh_facts {
                        assert_eq!(allocation.source, *source);
                        assert_eq!(loan.origin_place.source, *source);
                        assert_eq!(allocation.creation_span, loan.creation_span);
                        assert_eq!(
                            allocation.declaration_span,
                            loan.origin_place.binding.declaration_span
                        );
                        assert!(
                            loan.creation_span.start == text.find("&first").unwrap() as u32
                                || loan.creation_span.start == text.find("&second").unwrap() as u32
                        );
                    }
                    assert!(initial_ids.is_disjoint(final_origins));
                } else {
                    let borrowed = facts
                        .iter()
                        .find_map(|fact| match fact {
                            ReferenceOrigin::BorrowedFrom { parameter: 0, loan } => Some(loan),
                            _ => None,
                        })
                        .expect("parameter origin");
                    assert_eq!(borrowed.origin_place.source, *source);
                    assert_eq!(
                        borrowed.origin_place.binding.declaration_span.start,
                        text.find("fn(input").unwrap() as u32 + 3
                    );
                    assert_eq!(
                        borrowed.creation_span,
                        borrowed.origin_place.binding.declaration_span
                    );
                    assert_eq!(borrowed.mode, ScalarReferenceMutability::Shared);
                    let null_span = facts
                        .iter()
                        .find_map(|fact| match fact {
                            ReferenceOrigin::Null { span } => Some(*span),
                            _ => None,
                        })
                        .expect("null alternative");
                    assert_eq!(null_span.start, text.rfind("null;").unwrap() as u32);
                    assert!(initial_ids.is_disjoint(final_origins));
                }
            }
        }
}

#[test]
fn branch_only_inferred_reference_cannot_be_read_after_join() {
    let text = "%%start\n*u8(u8, bool) choose = fn(value, flag) { if (flag) { inferred = &value; }; inferred };\n%%end";
    let (single, project) = both_reference_functions(text);
    let read_start = text.rfind("inferred };").unwrap() as u32;
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0001"
                    && diagnostic
                        .labels
                        .iter()
                        .any(|label| label.span.range == ByteSpan::new(read_start, read_start + 8))
            }),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn straight_line_reference_cfg_has_source_backed_fresh_forward_reborrow_and_null() {
    let text = "%%start\n*u8(u8) fresh = fn(value) { &value };\n*u8(*u8) forward = fn(input) { *u8 alias = input; alias };\n*!u8(*!u8) reborrow = fn(parent) { &!(*parent) };\n*u8() nullable = fn { null };\n*u8(u8) nested = fn(value) { unsafe { u8 local = value; &local } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics, source) in [
        (
            &single.program.items,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let functions = items
            .iter()
            .map(|item| match item {
                ScalarItem::Function(function) => function,
                _ => panic!("function"),
            })
            .collect::<Vec<_>>();
        for function in &functions {
            let cfg = &function.reference_cfg;
            for (index, point) in cfg.points.iter().enumerate() {
                assert_eq!(point.id, CfgPointId(index));
                assert_eq!(&point.source_span.source, source);
                assert_eq!(
                    point.successors,
                    if index + 1 == cfg.points.len() {
                        vec![]
                    } else {
                        vec![CfgPointId(index + 1)]
                    }
                );
            }
            assert!(matches!(cfg.points[0].kind, CfgPointKind::Entry));
        }
        let fresh = functions[0];
        let address = fresh.reference_cfg.points.iter().find(|point| matches!(&point.kind, CfgPointKind::Borrow { loan, .. } if loan.creation_span.start == text.find("&value").unwrap() as u32)).expect("fresh borrow");
        let ReferenceOrigin::Fresh { allocation, loan } = sole_fact(fresh, address) else {
            panic!("fresh fact")
        };
        assert_eq!(allocation.creation_span, loan.creation_span);
        assert_eq!(&allocation.source, source);
        assert_eq!(
            allocation.declaration_span,
            ByteSpan::new(
                text.find("fn(value)").unwrap() as u32 + 3,
                text.find("fn(value)").unwrap() as u32 + 8
            )
        );
        assert_eq!(
            loan.origin_place.binding.declaration_span,
            allocation.declaration_span
        );
        assert_eq!(
            fresh.reference_cfg.points.last().unwrap().possible_origins,
            address.possible_origins
        );

        let forward = functions[1];
        let read = forward
            .reference_cfg
            .points
            .iter()
            .find(|point| {
                matches!(point.kind, CfgPointKind::Read { .. })
                    && point.source_span.range.start
                        == text
                            .find("alias }; ")
                            .unwrap_or(text.find("alias };").unwrap())
                            as u32
            })
            .expect("forwarded read");
        let ReferenceOrigin::BorrowedFrom { parameter: 0, loan } = sole_fact(forward, read) else {
            panic!("forwarded parameter")
        };
        assert_eq!(loan.creation_span, loan.origin_place.declaration_span);
        assert!(loan.parent.is_none());
        assert_eq!(
            forward
                .reference_cfg
                .points
                .last()
                .unwrap()
                .possible_origins,
            read.possible_origins
        );

        let reborrow = functions[2];
        let child = reborrow.reference_cfg.points.iter().find(|point| matches!(&point.kind, CfgPointKind::Borrow { loan, .. } if loan.creation_span.start == text.find("&!(*parent)").unwrap() as u32)).expect("child loan");
        let ReferenceOrigin::BorrowedFrom { parameter: 0, loan } = sole_fact(reborrow, child)
        else {
            panic!("reborrow origin")
        };
        assert_eq!(loan.mode, ScalarReferenceMutability::Mutable);
        assert_eq!(loan.creation_span, child.source_span.range);
        assert_eq!(loan.origin_place.source, *source);
        assert_eq!(
            loan.parent.as_ref().unwrap().creation_span,
            loan.origin_place.declaration_span
        );
        assert_eq!(
            reborrow
                .reference_cfg
                .points
                .last()
                .unwrap()
                .possible_origins,
            child.possible_origins
        );

        let nullable = functions[3];
        let point = nullable
            .reference_cfg
            .points
            .iter()
            .find(|point| point.source_span.range.start == text.find("null };").unwrap() as u32)
            .expect("null point");
        assert!(
            matches!(sole_fact(nullable, point), ReferenceOrigin::Null { span } if *span == point.source_span.range)
        );
        assert_eq!(
            nullable
                .reference_cfg
                .points
                .last()
                .unwrap()
                .possible_origins,
            point.possible_origins
        );

        let nested = functions[4];
        let local = nested.reference_cfg.points.iter().find(|point| matches!(&point.kind, CfgPointKind::Borrow { loan, .. } if loan.creation_span.start == text.find("&local").unwrap() as u32)).expect("nested loan");
        let ReferenceOrigin::Fresh { allocation, loan } = sole_fact(nested, local) else {
            panic!("nested fresh")
        };
        assert_eq!(
            allocation.declaration_span,
            ByteSpan::new(
                text.find("local = value").unwrap() as u32,
                text.find("local = value").unwrap() as u32 + 5
            )
        );
        assert_ne!(loan.origin_place.block_span, nested.body.span);
        assert_eq!(
            nested.reference_cfg.points.last().unwrap().possible_origins,
            local.possible_origins
        );
    }
}

#[test]
fn assignment_reads_rhs_before_write_and_shadowed_binding_is_diagnosed() {
    let text = "%%start\n*u8(u8, u8) f = fn(left, right) { *u8 alias = &left; alias = &right; alias };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let cfg = &function.reference_cfg;
        let borrows = cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Borrow { .. }))
            .collect::<Vec<_>>();
        assert_eq!(borrows.len(), 2);
        assert_ne!(borrows[0].possible_origins, borrows[1].possible_origins);
        let last = cfg
            .points
            .iter()
            .rev()
            .find(|point| matches!(point.kind, CfgPointKind::Read { .. }))
            .expect("final read");
        assert_eq!(last.possible_origins, borrows[1].possible_origins);
        assert!(
            matches!(sole_fact(function, last), ReferenceOrigin::Fresh { allocation, .. } if allocation.creation_span.start == text.find("&right").unwrap() as u32)
        );
    }
    let shadow = "%%start\n*u8(u8) f = fn(value) { *u8 alias = &value; unsafe { u8 inner = value; *u8 alias = &inner; alias }; alias };\n%%end";
    let (single, project) = both_reference_functions(shadow);
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0002"),
            "{diagnostics:?}"
        );
    }
    for (items, source) in [
        (&single.program.items, &single.program.source),
        (
            &project.project.modules[0].items,
            &project.project.modules[0].source,
        ),
    ] {
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let mut function = function.clone();
        assert!(record_function_reference_origins(
            source,
            &mut function,
            &HashSet::new(),
            &[],
            &[],
            &[],
            &HashMap::new()
        )
        .is_empty());
        let reads = function
            .reference_cfg
            .points
            .iter()
            .filter(|point| {
                matches!(point.kind, CfgPointKind::Read { .. })
                    && point.source_span.range.start >= shadow.find("&inner").unwrap() as u32
            })
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), 2);
        let CfgPointKind::Read { binding: inner } = &reads[0].kind else {
            panic!("inner")
        };
        let CfgPointKind::Read { binding: outer } = &reads[1].kind else {
            panic!("outer")
        };
        assert_ne!(inner, outer);
        assert_ne!(inner.block_span, outer.block_span);
        assert!(
            matches!(sole_fact(&function, reads[0]), ReferenceOrigin::Fresh { allocation, .. } if allocation.creation_span.start == shadow.find("&inner").unwrap() as u32)
        );
        assert!(
            matches!(sole_fact(&function, reads[1]), ReferenceOrigin::Fresh { allocation, .. } if allocation.creation_span.start == shadow.find("&value").unwrap() as u32)
        );
    }
}

#[test]
fn unresolved_name_and_deliberately_empty_internal_origin_are_explicit() {
    let text = "%%start\n*u8() f = fn { &missing };\n%%end";
    let (single, project) = both_reference_functions(text);
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0001"
                    && diagnostic.labels.iter().any(
                        |label| label.span.range.start == text.find("missing").unwrap() as u32
                    )),
            "{diagnostics:?}"
        );
    }
    let text = "%%start\n*u8(*u8) f = fn(parent) { parent };\n%%end";
    let result = validate_text(text);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Function(function) = &result.program.items[0] else {
        panic!("function")
    };
    let mut builder = ReferenceFlowBuilder {
        source: result.program.source.clone(),
        function_span: function.span,
        names: BTreeMap::new(),
        module_names: HashSet::new(),
        call_sites: Vec::new(),
        origins: HashMap::new(),
        checked: HashSet::new(),
        binding_types: HashMap::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        generic_parameters: BTreeSet::new(),
        assignment_outputs: HashMap::new(),
        dereference_points: HashMap::new(),
        cfg: ScalarReferenceCfg {
            facts: Vec::new(),
            points: Vec::new(),
            initial_points: Vec::new(),
            allocations: BTreeSet::new(),
            specialized_policies: Vec::new(),
        },
        next_scope: 0,
        cursor: None,
    };
    let binding = ReferenceBindingId {
        source: result.program.source.clone(),
        function_span: function.span,
        declaration_span: function.parameter_spans[0],
        block_span: function.body.span,
        kind: ReferenceBindingKind::Declared,
    };
    builder.names.insert("parent".to_owned(), binding.clone());
    builder.checked.insert(binding.clone());
    builder.origins.insert(binding.clone(), BTreeSet::new());
    let error = builder
        .expression(
            &function.body.final_output_values[0].value,
            ReferenceScopeId(0),
        )
        .expect_err("missing fact");
    assert!(matches!(&error, ReferenceAnalysisError::MissingOrigin {
            binding_id,
            reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
            ..
        } if binding_id == &binding));
    assert_eq!(
        error.source_span().range.start,
        text.rfind("parent };").unwrap() as u32
    );
    builder.names.clear();
    let error = builder
        .expression(
            &function.body.final_output_values[0].value,
            ReferenceScopeId(0),
        )
        .expect_err("missing resolved binding");
    assert!(
        matches!(error, ReferenceAnalysisError::MissingBinding { name, .. } if name == "parent")
    );
}

#[test]
fn inferred_assignment_declares_at_target_and_later_assignments_reuse_identity() {
    let text =
        "%%start\n*u8(u8) f = fn(value) { inferred = &value; inferred = &value; inferred };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics, source) in [
        (
            &single.program.items,
            &single.diagnostics,
            &single.program.source,
        ),
        (
            &project.project.modules[0].items,
            &project.diagnostics,
            &project.project.modules[0].source,
        ),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let assignments = function
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Assign { target, .. } => Some((point, target)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(assignments.len(), 2);
        let declaration_span = ByteSpan::new(
            text.find("inferred =").unwrap() as u32,
            text.find("inferred =").unwrap() as u32 + "inferred".len() as u32,
        );
        assert_eq!(assignments[0].1.declaration_span, declaration_span);
        assert_eq!(assignments[0].1.binding, assignments[1].1.binding);
        assert_eq!(&assignments[0].1.source, source);
        assert_eq!(assignments[0].1.function_span, function.span);
        assert_eq!(assignments[0].1.block_span, function.body.span);
        let read = function.reference_cfg.points.iter().find(|point| matches!(&point.kind, CfgPointKind::Read { binding } if binding == &assignments[0].1.binding)).expect("inferred read");
        assert_eq!(read.possible_origins, assignments[1].0.possible_origins);
        assert!(
            matches!(sole_fact(function, read), ReferenceOrigin::Fresh { allocation, .. } if allocation.creation_span.start == text.rfind("&value").unwrap() as u32)
        );
    }
}

#[test]
fn checked_assignment_captures_rhs_before_old_loan_is_replaced() {
    let text =
        "%%start\n*u8(u8, u8) f = fn(first, second) { *u8 r = &first; r = &second; r };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let cfg = &function.reference_cfg;
        let assignment = cfg.points.iter().find(|point| {
                matches!(&point.kind, CfgPointKind::Assign { target, .. } if target.binding.declaration_span.start == text.find("r = &first").unwrap() as u32)
            }).expect("assignment");
        let CfgPointKind::Assign {
            target,
            rhs_point,
            previous_origins,
            previous_loans,
        } = &assignment.kind
        else {
            unreachable!()
        };
        let new_borrow = text.find("&second").unwrap() as u32;
        assert_eq!(cfg.points[rhs_point.0].source_span.range.start, new_borrow);
        assert!(rhs_point.0 < assignment.id.0);
        assert_eq!(
            target.declaration_span.start,
            text.find("r = &first").unwrap() as u32
        );
        assert!(target.projections.is_empty());
        assert_eq!(previous_origins.len(), 1);
        assert_eq!(previous_loans.len(), 1);
        assert_eq!(
            previous_loans[0].creation_span.start,
            text.find("&first").unwrap() as u32
        );
        assert!(
            matches!(sole_fact(function, assignment), ReferenceOrigin::Fresh { loan, .. } if loan.creation_span.start == new_borrow)
        );
    }
}

#[test]
fn multiple_assignment_borrows_both_outputs_before_either_target_write() {
    let text = "%%start\n*u8(u8, u8) f = fn(first, second) { *u8 left = &first; *u8 right = &second; left, right = right, left; left };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let cfg = &function.reference_cfg;
        let assignments = cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Assign { .. }))
            .collect::<Vec<_>>();
        assert_eq!(assignments.len(), 2);
        let rhs_reads = cfg
            .points
            .iter()
            .filter(|point| {
                matches!(point.kind, CfgPointKind::Read { .. })
                    && point.source_span.range.start >= text.find("= right, left").unwrap() as u32
                    && point.source_span.range.start < text.find("; left };").unwrap() as u32
            })
            .collect::<Vec<_>>();
        assert_eq!(rhs_reads.len(), 2);
        assert!(rhs_reads.iter().all(|read| read.id.0 < assignments[0].id.0));
        for (assignment, read) in assignments.iter().zip(&rhs_reads) {
            let CfgPointKind::Assign {
                rhs_point,
                previous_origins,
                ..
            } = &assignment.kind
            else {
                unreachable!()
            };
            assert_eq!(rhs_point, &read.id);
            assert_eq!(previous_origins.len(), 1);
            assert_eq!(assignment.possible_origins, read.possible_origins);
        }
    }
}

#[test]
fn resolved_field_assignment_preserves_disjoint_checked_loan() {
    let text = "%%start\nstruct Bundle { u8 left; u8 right; }\n*u8() f = fn { Bundle pair = { .left = 1; .right = 2; }; *u8 reader = &pair.right; pair.left = 3; reader };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let function = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) => Some(function),
                _ => None,
            })
            .expect("function");
        let cfg = &function.reference_cfg;
        let assignment = cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::Assign { .. }))
            .expect("field write");
        let CfgPointKind::Assign {
            target,
            previous_origins,
            ..
        } = &assignment.kind
        else {
            unreachable!()
        };
        assert_eq!(target.projections.len(), 1);
        assert!(matches!(
            &target.projections[0],
            ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(_))
        ));
        assert!(previous_origins.is_empty());
        let reader = cfg
            .points
            .iter()
            .rev()
            .find(|point| matches!(point.kind, CfgPointKind::Read { .. }))
            .expect("reader");
        assert_eq!(reader.possible_origins.as_ref().expect("loan").len(), 1);
    }
}

#[test]
fn moving_allocation_records_live_loan_before_move_point() {
    let text = "%%start\n*u8(u64) f = fn(count) { u8[count] bytes; *u8 borrowed = &bytes[0]; u8[] moved = bytes; moved; borrowed };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message == "read or move of unavailable place"
                    && diagnostic.labels[0].span.range.start
                        == text.find("= bytes; moved").unwrap() as u32 + 2
            }),
            "{diagnostics:?}"
        );
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let cfg = &function.reference_cfg;
        let moved = cfg
            .points
            .iter()
            .find(|point| {
                matches!(point.kind, CfgPointKind::Move { .. })
                    && point.source_span.range.start
                        == text.find("= bytes; moved").unwrap() as u32 + 2
            })
            .expect("allocation move");
        let CfgPointKind::Move { place, owner, .. } = &moved.kind else {
            unreachable!()
        };
        assert_eq!(&place.binding, owner);
        let borrow = cfg.points.iter().find(|point| matches!(&point.kind, CfgPointKind::Borrow { place, .. } if place.binding == *owner)).expect("live loan");
        assert!(borrow.id.0 < moved.id.0);
        assert_eq!(
            borrow.source_span.range.start,
            text.find("&bytes[0]").unwrap() as u32
        );
        assert!(cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Read { binding } if binding.declaration_span.start == text.find("borrowed =").unwrap() as u32) && point.id.0 > moved.id.0));
    }
}

#[test]
fn constant_index_write_preserves_loan_to_other_element() {
    let text = "%%start\n*u8() f = fn { u8[2] bytes = [1, 2]; *u8 reader = &bytes[1]; bytes[0] = 3; reader };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let cfg = &function.reference_cfg;
        let assignment = cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::Assign { .. }))
            .expect("index assignment");
        let CfgPointKind::Assign {
            target,
            previous_origins,
            ..
        } = &assignment.kind
        else {
            unreachable!()
        };
        assert!(
            matches!(&target.projections[..], [ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. })] if value == &0.into())
        );
        assert!(previous_origins.is_empty());
        let final_read = cfg
            .points
            .iter()
            .rev()
            .find(|point| matches!(point.kind, CfgPointKind::Read { .. }))
            .expect("reader");
        assert_eq!(
            final_read
                .possible_origins
                .as_ref()
                .expect("checked loan")
                .len(),
            1
        );
    }
}

#[test]
fn raw_core_free_tracks_owner_release_in_both_validators() {
    let text = "%%start\nunit() release = fn { unsafe { *?u8 storage = core.alloc(1, 1); core.free(storage); core.free(storage); } };\n%%end";
    let (single, project) = both_reference_functions(text);
    let second = text.rfind("core.free(storage)").unwrap() as u32;
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start == second),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn projected_checked_assignment_transfers_rhs_to_field_and_preserves_sibling() {
    let text = "%%start\nstruct Holder { *u8 first; *u8 second; }\nunit(u8, u8, u8) update = fn(a, b, c) { Holder box = { .first = null; .second = null; }; box.first = &a; box.second = &b; box.first = &c; *u8 kept = box.second; *u8 replaced = box.first; kept; replaced; } ;\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        let function = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) => Some(function),
                _ => None,
            })
            .expect("function");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let reads = function
            .reference_cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReadPlace { .. }))
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), 2);
        for (read, origin) in reads.into_iter().zip(["&b", "&c"]) {
            let fact = sole_fact(function, read);
            assert!(
                matches!(fact, ReferenceOrigin::Fresh { loan, .. } if loan.creation_span.start == text.find(origin).unwrap() as u32),
                "{fact:?}"
            );
        }
    }
}

#[test]
fn projected_checked_swap_uses_both_original_rhs_origins() {
    let text = "%%start\nstruct Pair { *u8 left; *u8 right; }\nunit(u8, u8) swap = fn(a, b) { Pair box = { .left = null; .right = null; }; box.left = &a; box.right = &b; box.left, box.right = box.right, box.left; *u8 left = box.left; *u8 right = box.right; left; right; };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let function = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) => Some(function),
                _ => None,
            })
            .expect("function");
        let cfg = &function.reference_cfg;
        let swap_start = text.find("box.left, box.right =").unwrap() as u32;
        let writes = cfg
            .points
            .iter()
            .filter(|point| {
                matches!(point.kind, CfgPointKind::Assign { .. })
                    && point.source_span.range.start >= swap_start
            })
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 2);
        for (write, expected) in writes.iter().zip(["&b", "&a"]) {
            let CfgPointKind::Assign { rhs_point, .. } = &write.kind else {
                unreachable!()
            };
            assert!(rhs_point.0 < writes[0].id.0);
            assert!(
                matches!(sole_fact(function, write), ReferenceOrigin::Fresh { loan, .. }
                    if loan.creation_span.start == text.find(expected).unwrap() as u32)
            );
        }
    }
}

#[test]
fn projected_checked_constant_indices_preserve_disjoint_origins() {
    let text = "%%start\nunit(u8, u8, u8) update = fn(a, b, c) { (*u8)[2] refs = [null, null]; refs[0] = &a; refs[1] = &b; refs[0] = &c; *u8 other = refs[1]; *u8 current = refs[0]; other; current; };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let reads = function
            .reference_cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReadPlace { .. }))
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), 2);
        for (read, origin) in reads.into_iter().zip(["&b", "&c"]) {
            assert!(
                matches!(sole_fact(function, read), ReferenceOrigin::Fresh { loan, .. }
                    if loan.creation_span.start == text.find(origin).unwrap() as u32)
            );
        }
    }
}

#[test]
fn checked_dereference_field_write_transfers_checked_rhs() {
    let text = "%%start\nstruct Bag { *u8 item; }\nunit(u8) update = fn(value) { Bag parcel = { .item = null; }; *!Bag writer = &!parcel; (*writer).item = &value; *u8 reader = parcel.item; reader; };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let function = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) => Some(function),
                _ => None,
            })
            .expect("function");
        let cfg = &function.reference_cfg;
        let write = cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::AssignThrough { .. }))
            .expect("dereference write");
        let CfgPointKind::AssignThrough { candidates, .. } = &write.kind else {
            unreachable!()
        };
        assert_eq!(candidates.len(), 1);
        assert!(
            matches!(sole_fact(function, write), ReferenceOrigin::Fresh { loan, .. }
                if loan.creation_span.start == text.find("&value").unwrap() as u32)
        );
    }
}

#[test]
fn core_invalidate_expires_owner_and_rebind_restores_selected_backing() {
    let invalid = "%%start\nunit(u64) expire = fn(count) { u8[count] bytes; unsafe { core.invalidate(&bytes); }; bytes[0]; };\n%%end";
    let (single, project) = both_reference_functions(invalid);
    let read = invalid.rfind("bytes[0]").unwrap() as u32;
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"
                    && diagnostic.labels[0].span.range.start == read),
            "{diagnostics:?}"
        );
    }
    let rebound = "%%start\nunit(u64, *?u8) update = fn(count, raw) { u8[count] bytes; u8[count] other; unsafe { core.invalidate(&bytes); core.rebind(&bytes, raw, count); }; bytes[0]; other[0]; };\n%%end";
    let (single, project) = both_reference_functions(rebound);
    for diagnostics in [&single.diagnostics, &project.diagnostics] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}

#[test]
fn joined_checked_assignment_preserves_all_reachable_targets() {
    let text = "%%start\nunit(u8, u8, bool) update = fn(first, second, choose) { *!u8 selected = &!first; if (choose) { selected = &!second; }; *selected = 3; selected; } ;\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let assignment = function
            .reference_cfg
            .points
            .iter()
            .find(|point| matches!(point.kind, CfgPointKind::AssignThrough { .. }))
            .expect("checked place assignment");
        let CfgPointKind::AssignThrough {
            candidates,
            previous_origins,
            rhs_point,
            pointer_point,
            ..
        } = &assignment.kind
        else {
            unreachable!()
        };
        assert_eq!(candidates.len(), 2);
        assert!(previous_origins.is_empty());
        assert!(rhs_point.0 < pointer_point.0);
        assert!(pointer_point.0 < assignment.id.0);
        assert_eq!(
            assignment.source_span.range.start,
            text.find("*selected = 3").unwrap() as u32 + 1
        );
        assert!(candidates
            .iter()
            .any(|candidate| candidate.place.binding.declaration_span.start
                == text.find("fn(first").unwrap() as u32 + 3));
        assert!(candidates
            .iter()
            .any(|candidate| candidate.place.binding.declaration_span.start
                == text.find("second, choose").unwrap() as u32));
    }
}

#[test]
fn null_reborrow_keeps_the_original_and_conflicting_spans() {
    let text = "%%start\n*u8() f = fn { *u8 empty = null; &*empty };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let point = function.reference_cfg.points.last().expect("return point");
        assert!(
            matches!(sole_fact(function, point), ReferenceOrigin::Invalid { origin_span, conflict_span }
                if origin_span.start == text.find("null;").unwrap() as u32
                    && conflict_span.start == text.find("&*empty").unwrap() as u32),
            "{:?}",
            sole_fact(function, point)
        );
    }
}

#[test]
fn unsafe_statement_and_unit_output_remain_statements_in_both_pipelines() {
    for (source_text, final_output_count, terminated_items) in [
        (
            "%%start\n*u8(u8) f = fn(v) { unsafe { &v }; &v };\n%%end",
            1,
            &[true, false][..],
        ),
        (
            "%%start\nunit(u8) f = fn(v) { unsafe { &v }; };\n%%end",
            0,
            &[true][..],
        ),
        (
            "%%start\nunit(u8) f = fn(v) { unsafe { &v; } };\n%%end",
            1,
            &[false][..],
        ),
    ] {
        let result = validate_text(source_text);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let module_source = module_source("src/main.w");
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                module_source.clone(),
                module_from_text(module_source.clone(), source_text).items,
                Vec::new(),
            )],
            vec![module_source],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        for items in [&result.program.items, &project.project.modules[0].items] {
            let ScalarItem::Function(function) = &items[0] else {
                panic!("expected function");
            };
            let body = &function.body;
            assert_eq!(body.items.len(), body.terminated_items.len());
            assert_eq!(body.items.len(), body.expressions.len());
            assert_eq!(body.final_output_values.len(), final_output_count);
            assert_eq!(body.terminated_items, terminated_items);
        }
    }
}

#[test]
fn derives_zero_payload_enum_tags_and_equality() {
    let result = validate_text(
            "%%start\nenum Status { first; second; third; }\nStatus value = Status::second;\nbool equal = value == Status::second;\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.program.enums[0].variants[0].tag, 0);
    assert_eq!(result.program.enums[0].variants[1].tag, 1);
    assert_eq!(result.program.enums[0].variants[2].tag, 2);
}

#[test]
fn validates_mutable_typed_place_writes_and_assignment_distinctness() {
    let valid = validate_text(
            "%%start\nstruct Record { u8 value; u8[3] bytes; }\nu8[3] values = [1, 2, 3];\nRecord holder = { .value = 4; .bytes = [5, 6, 7]; };\n*!u8 writer = &!values[1];\nvalues[0] = 8;\nholder.value = 9;\n*writer = 10;\nholder.bytes[2] = 11;\nvalues[0], values[2] = 12, 13;\n%%end",
        );
    assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);

    let dynamic = validate_text(
        "%%start\nu8[2] values = [1, 2];\nu64 index = 0;\nvalues[index], values[1] = 3, 4;\n%%end",
    );
    assert!(dynamic.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "B0002"
            && diagnostic.message == "assignment targets require a distinctness proof"
    }));

    let shared = validate_text(
        "%%start\nu8[2] values = [1, 2];\n*u8 shared = &values[0];\n*shared = 3;\n%%end",
    );
    assert!(shared.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "B0003"
            && diagnostic.message == "assignment dereference requires a mutable checked reference"
    }));
}

#[test]
fn validates_runtime_array_callable_transport() {
    let validation = validate_text(
            "%%start\nu8[]() make = fn { u64 length = 1; u8[length] bytes; bytes };\nu8[](u8[]) relay = fn(values) { values };\nu8[] value = make();\nu8[] alias = relay(value);\n%%end",
        );
    assert!(
        validation.diagnostics.is_empty(),
        "diagnostics: {:#?}\nprogram: {:#?}",
        validation.diagnostics,
        validation.program
    );
}

#[test]
fn validates_runtime_array_binding_move_and_top_level_materialization() {
    let validation = validate_text(
            "%%start\nu64 length = 1;\nu8[length] first;\nu8[] second = first;\nsecond[0] = 7;\nu8 value = second[0];\n%%end",
        );
    assert!(
        validation.diagnostics.is_empty(),
        "diagnostics: {:#?}\nprogram: {:#?}",
        validation.diagnostics,
        validation.program
    );
}

#[test]
fn enforces_canonical_assignment_targets_and_writable_place_paths() {
    for text in [
            "%%start\nu8 value = 0;\nvalue, value = 1, 2;\n%%end",
            "%%start\nstruct Record { u8 value; }\nRecord holder = { .value = 0; };\nholder.value, holder.value = 1, 2;\n%%end",
            "%%start\nu8[2] items = [0, 0];\nitems[0], items[0] = 1, 2;\n%%end",
        ] {
            let result = validate_text(text);
            assert!(result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0002" && diagnostic.message == "duplicate assignment target"
            }));
        }
    for text in [
            "%%start\nstruct Record { u8 first; u8 second; }\nRecord holder = { .first = 0; .second = 0; };\nholder.first, holder.second = 1, 2;\n%%end",
            "%%start\nu8[2] items = [0, 0];\nitems[0], items[1] = 1, 2;\n%%end",
        ] {
            let result = validate_text(text);
            assert!(
                result.diagnostics.is_empty(),
                "{text}: {:?}",
                result.diagnostics
            );
        }
    for text in [
        "%%start\nu8[2] items = [0, 0];\nu64 index = 0;\nitems[index], items[0] = 1, 2;\n%%end",
        "%%start\nu8[2] items = [0, 0];\nu64 index = 0;\nitems[0], items[index] = 1, 2;\n%%end",
    ] {
        let result = validate_text(text);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0002"
                && diagnostic.message == "assignment targets require a distinctness proof"
        }));
    }

    let valid = validate_text(
            "%%start\nstruct Record { u8 value; u8[2] bytes; }\nRecord holder = { .value = 0; .bytes = [0, 0]; };\n*!Record writer = &!holder;\n(*writer).value = 1;\n(*writer).bytes[0] = 2;\n%%end",
        );
    assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);

    for target in ["(*raw).value", "(*raw).bytes[0]"] {
        let text = format!(
                "%%start\nstruct Record {{ u8 value; u8[2] bytes; }}\nRecord holder = {{ .value = 0; .bytes = [0, 0]; }};\nunsafe {{ *?Record raw = &?holder; {target} = 1; }};\n%%end"
            );
        let result = validate_text(&text);
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.message == "assignment through raw pointer dereference is not supported"
            })
            .expect("raw write diagnostic");
        let start = text.find(target).expect("raw assignment target") as u32;
        assert_eq!(diagnostic.code, "B0003");
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + target.len() as u32)
        );
    }

    for text in [
            "%%start\nu8[2] values = [1, 2];\n*!(u8[2]) writer = &!values;\n*writer = [3, 4];\n%%end",
            "%%start\nstruct Record { u8[2] bytes; }\nRecord holder = { .bytes = [1, 2]; };\nholder.bytes = [3, 4];\n%%end",
        ] {
            let result = validate_text(text);
            assert!(result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.message == "assignment place requires a scalar leaf type"
            }));
        }
}

#[test]
fn enforces_assignment_target_and_writable_path_rules_in_projects() {
    let validate_project_text = |text: &str| {
        let main_source = module_source("src/main.w");
        let main = module_from_text(main_source.clone(), text);
        validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(main, Vec::new())],
            vec![main_source],
        ))
    };
    let duplicates = validate_project_text(
            "%%start\nstruct Record { u8 first; u8 second; }\nu8 value = 0;\nRecord record = { .first = 0; .second = 0; };\nu8[2] items = [0, 0];\nvalue, value = 1, 2;\nrecord.first, record.first = 1, 2;\nitems[0], items[0] = 1, 2;\n%%end",
        );
    assert_eq!(
        duplicates
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message == "duplicate assignment target")
            .count(),
        3,
        "{:?}",
        duplicates.diagnostics
    );
    let distinct = validate_project_text(
            "%%start\nstruct Record { u8 first; u8 second; }\nRecord record = { .first = 0; .second = 0; };\nu8[2] items = [0, 0];\nrecord.first, record.second = 1, 2;\nitems[0], items[1] = 1, 2;\n%%end",
        );
    assert!(
        distinct.diagnostics.is_empty(),
        "{:?}",
        distinct.diagnostics
    );
    for text in [
        "%%start\nu8[2] items = [0, 0];\nu64 index = 0;\nitems[index], items[0] = 1, 2;\n%%end",
        "%%start\nu8[2] items = [0, 0];\nu64 index = 0;\nitems[0], items[index] = 1, 2;\n%%end",
    ] {
        let result = validate_project_text(text);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0002"
                && diagnostic.message == "assignment targets require a distinctness proof"
        }));
    }
    let valid = validate_project_text(
            "%%start\nstruct Record { u8 value; u8[2] bytes; }\nRecord holder = { .value = 0; .bytes = [0, 0]; };\n*!Record writer = &!holder;\n(*writer).value = 1;\n(*writer).bytes[0] = 2;\n%%end",
        );
    assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
    let raw_text = "%%start\nstruct Record { u8 value; u8[2] bytes; }\nRecord record = { .value = 0; .bytes = [0, 0]; };\nunsafe { *?Record raw = &?record; (*raw).value = 1; (*raw).bytes[0] = 2; };\n%%end";
    let raw = validate_project_text(raw_text);
    for target in ["(*raw).value", "(*raw).bytes[0]"] {
        let start = raw_text.find(target).expect("raw assignment target") as u32;
        let diagnostic = raw
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.message == "assignment through raw pointer dereference is not supported"
                    && diagnostic.labels[0].span.range
                        == ByteSpan::new(start, start + target.len() as u32)
            })
            .expect("raw write diagnostic");
        assert_eq!(diagnostic.code, "B0003");
    }
    let aggregate = validate_project_text(
        "%%start\nu8[2] values = [1, 2];\n*!(u8[2]) writer = &!values;\n*writer = [3, 4];\n%%end",
    );
    assert!(aggregate.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "assignment place requires a scalar leaf type"
    }));
}

#[test]
fn derives_and_types_fixed_array_index_places_and_addresses() {
    let text = "%%start\nstruct Record { u8[2] bytes; }\nu8[2][2] matrix = [[1, 2], [3, 4]];\nu64 row = 1;\nu64 column = 0;\nRecord holder = { .bytes = [5, 6]; };\nu8 nested = matrix[row][column];\nu8 field = holder.bytes[column];\n*u8 shared = &matrix[row][column];\n*!u8 mutable = &!holder.bytes[column];\nunsafe { *?u8 raw = &?matrix[row][column]; };\n%%end";
    let result = validate_text(text);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(nested) = &result.program.items[4] else {
        panic!("nested indexed read")
    };
    let ScalarExpression::IndexedRead {
        place: ScalarPlace::Index {
            base, index_span, ..
        },
        ..
    } = &nested.value
    else {
        panic!("nested indexed read place")
    };
    let index_start = text.find("column];").expect("column") as u32;
    assert_eq!(*index_span, ByteSpan::new(index_start, index_start + 6));
    assert!(matches!(base.as_ref(), ScalarPlace::Index { .. }));
    for item in [6, 7] {
        let ScalarItem::Binding(binding) = &result.program.items[item] else {
            panic!("indexed address binding")
        };
        assert!(matches!(
            binding.value,
            ScalarExpression::CheckedAddress {
                place: ScalarPlace::Index { .. },
                ..
            }
        ));
    }
    let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
        &result.program.items[8]
    else {
        panic!("unsafe indexed raw address")
    };
    assert!(matches!(
        block.items[0],
        ScalarBlockItem::LocalBinding(ScalarBinding {
            value: ScalarExpression::RawAddress {
                place: ScalarPlace::Index { .. },
                ..
            },
            ..
        })
    ));

    let invalid = validate_text(
            "%%start\nu8[1] bytes = [1];\ni32 index = 0;\nu8 value = bytes[index];\nu8 wrong = index[0];\nu8[0] empty = [];\nu8 accepted = empty[0];\n%%end",
        );
    assert!(invalid
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "expression type does not match expected type"));
    assert!(invalid
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "indexed place requires a fixed array"));
    assert!(invalid
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.message != "index is outside the array"));
}

#[test]
fn derives_pointer_struct_layouts_from_the_target() {
    let text = "%%start\nstruct Pair {\n\t*?u8 address;\n\tu32 count;\n}\n%%end";
    let parsed = parse_source(source(), text.to_owned(), &[]);

    let wasm = derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::WASM32);
    let native = derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::NATIVE64);

    assert_eq!(
        wasm.program.structs[0].layout,
        Some(ScalarLayout {
            size: 8,
            alignment: 4
        })
    );
    assert_eq!(wasm.program.structs[0].fields[1].offset, Some(4));
    assert_eq!(
        native.program.structs[0].layout,
        Some(ScalarLayout {
            size: 16,
            alignment: 8
        })
    );
    assert_eq!(native.program.structs[0].fields[1].offset, Some(8));
}

#[test]
fn derives_unit_if_statement_and_requires_bool_condition_span() {
    let text =
        "%%start\nunit() touch = fn { 1; };\nunit() run = fn { if (true) { touch(); }; };\n%%end";
    let result = validate_text(text);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Function(function) = &result.program.items[1] else {
        panic!("unit conditional function")
    };
    assert!(matches!(
        function.body.items[0],
        ScalarBlockItem::Expression(ScalarExpression::UnitIf { .. })
    ));

    let invalid = validate_text(
        "%%start\nunit() touch = fn { 1; };\nunit() run = fn { if (1) { touch(); }; };\n%%end",
    );
    let condition = invalid
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message == "expression type does not match expected type")
        .expect("bool condition diagnostic");
    let start = "%%start\nunit() touch = fn { 1; };\nunit() run = fn { if (".len() as u32;
    assert_eq!(
        condition.labels[0].span.range,
        ByteSpan::new(start, start + 1)
    );
}

#[test]
fn rejects_unit_if_in_value_contexts_at_the_conditional_span() {
    let cases = [
            "%%start\nunit() touch = fn { 1; };\nunit value = if (true) { touch(); };\n%%end",
            "%%start\nunit() touch = fn { 1; };\nunit() value = fn { if (true) { touch(); } };\n%%end",
            "%%start\nunit() touch = fn { 1; };\nunit(unit) accept = fn(value) { touch(); };\naccept(if (true) { touch(); });\n%%end",
            "%%start\nunit() touch = fn { 1; };\nunit target = touch();\ntarget = if (true) { touch(); };\n%%end",
            "%%start\nunit() touch = fn { 1; };\ni32 value = (if (true) { touch(); }) + 1;\n%%end",
        ];
    for text in cases {
        let result = validate_text(text);
        let start = text.find("if (true)").expect("conditional") as u32;
        let end = start + "if (true) { touch(); }".len() as u32;
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.message == "if without else is only valid as a unit statement"
                && diagnostic.labels[0].span.range == ByteSpan::new(start, end)
        }));
    }
}

#[test]
fn derives_and_validates_wasi_fd_write_with_typed_utf8() {
    let result = validate_text(
            "%%start\nwasi = extern wasm \"wasi_snapshot_preview1\" { i32(i32, i32) fd_write; };\ni32 out = wasi.fd_write(1, 1);\ni32 err = wasi.fd_write(2, 2);\n%%end",
        );
    assert!(
        result.diagnostics.is_empty(),
        "{:?}\n{:?}",
        result.diagnostics,
        result.program
    );
    let ScalarItem::Extern(extern_decl) = &result.program.items[0] else {
        panic!("extern item");
    };
    assert_eq!(
        extern_decl.actual_module,
        ScalarExternModule::Valid("wasi_snapshot_preview1".to_owned())
    );
    assert_eq!(extern_decl.functions[0].name, "fd_write");
    assert!(!extern_decl.functions[0].unsafe_marker);
    let ScalarItem::Binding(binding) = &result.program.items[1] else {
        panic!("binding item");
    };
    assert!(matches!(binding.value, ScalarExpression::Call { .. }));
    assert!(matches!(binding.declared_type, ScalarType::I32));
}

#[test]
fn validates_public_read_into_signature_and_hides_preview1_read_details() {
    let preview_source = module_source("src/wasi/preview1.w");
    let preview = module_from_text(
            preview_source.clone(),
            "%%start\nstruct _ReadIovec { *?u8 data; u32 length; }\nstruct _Nread { u32 value; }\n_wasi = extern wasm \"wasi_snapshot_preview1\" { unsafe i32(i32, *?_ReadIovec, i32, *?_Nread) _fd_read; };\n(u64, bool)(*?u8, u64) _fd_read_once = fn(destination, capacity) { _ReadIovec _iovec = { .data = destination; .length = core.int_trunc<u32>(capacity); }; _Nread _byte_count = { .value = 0; }; i32 _result = unsafe { _wasi._fd_read(0, &?_iovec, 1, &?_byte_count) }; u64 reported = 0; bool complete = false; if (_result == 0) { reported = core.int_extend<u64>(_byte_count.value); complete = true; } else { reported = 0; complete = false; }; reported, complete };\n(u64, bool)(*?u8, u64) read_into = fn(destination, capacity) { _fd_read_once(destination, capacity) };\n%%end",
        );
    let bootstrap_source = module_source("src/bootstrap.w");
    let bootstrap = module_from_text(
            bootstrap_source.clone(),
            "%%start\npreview1 = namespace std \"wasi/preview1.w\";\n(u64, bool)(*?u8, u64) read_into = fn(destination, capacity) { preview1.read_into(destination, capacity) };\n%%end",
        );
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nstd = namespace std \"bootstrap.w\";\n*?u8 destination = null;\nu64 capacity = 4;\nu64 count = 0;\nbool complete = false;\ncount, complete = std.read_into(destination, capacity);\n%%end",
        );
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                main,
                vec![ScalarNamespaceBinding {
                    binding: "std".to_owned(),
                    target: bootstrap_source.clone(),
                    span: ByteSpan::new(0, 0),
                }],
            ),
            ScalarModule::from_program(
                bootstrap,
                vec![ScalarNamespaceBinding {
                    binding: "preview1".to_owned(),
                    target: preview_source.clone(),
                    span: ByteSpan::new(0, 0),
                }],
            ),
            ScalarModule::from_program(preview, Vec::new()),
        ],
        vec![main_source, bootstrap_source, preview_source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let ScalarItem::Binding(count) = &validation.project.modules[0].items[3] else {
        panic!("read count binding")
    };
    assert_eq!(count.declared_type, ScalarType::U64);
    let ScalarItem::Binding(complete) = &validation.project.modules[0].items[4] else {
        panic!("read complete binding")
    };
    assert_eq!(complete.declared_type, ScalarType::Bool);

    assert!(validation.project.modules[1]
        .members
        .contains_key("read_into"));
    assert!(!validation.project.modules[1]
        .members
        .contains_key("fd_read_once"));
    assert!(!validation.project.modules[2]
        .members
        .contains_key("_fd_read"));
    assert!(!validation.project.modules[2]
        .members
        .contains_key("_fd_read_once"));
}

#[test]
fn rejects_undeclared_extern_type_arguments_and_preserves_valid_calls() {
    let text = "%%start\nenv = extern wasm \"helper\" { (u64, bool)() read; };\nu64 first, bool second = env.read<u32>();\n%%end";
    let rejected = validate_text(text);
    assert_eq!(rejected.diagnostics.len(), 1, "{:?}", rejected.diagnostics);
    let diagnostic = &rejected.diagnostics[0];
    assert_eq!(diagnostic.code, "B0004");
    assert_eq!(
        diagnostic.message,
        "generic argument arity does not match callable declaration"
    );
    let start = text.find("env.read<u32>()").expect("extern call") as u32;
    assert_eq!(
        diagnostic.labels[0].span.range,
        ByteSpan::new(start, start + 15)
    );

    let ordinary = validate_text(
        "%%start\nenv = extern wasm \"helper\" { u64() read; };\nu64 value = env.read();\n%%end",
    );
    assert!(
        ordinary.diagnostics.is_empty(),
        "{:?}",
        ordinary.diagnostics
    );

    let generic = validate_text(
            "%%start\ngeneric T;\nT(T) identity = fn(value) { value };\nu64 result = identity<u64>(1);\n%%end",
        );
    assert!(generic.diagnostics.is_empty(), "{:?}", generic.diagnostics);
}

#[test]
fn rejects_undeclared_project_extern_type_arguments_at_call_span() {
    let main_source = module_source("src/main.w");
    let library_source = module_source("src/library.w");
    let main_text = "%%start\nlibrary = namespace app \"src/library.w\";\nu64 first, bool second = library.read<u32>();\n%%end";
    let main = module_from_text(main_source.clone(), main_text);
    let library = module_from_text(
        library_source.clone(),
        "%%start\nenv = extern wasm \"helper\" { (u64, bool)() read; };\n%%end",
    );
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                main,
                vec![ScalarNamespaceBinding {
                    binding: "library".to_owned(),
                    target: library_source.clone(),
                    span: ByteSpan::new(8, 15),
                }],
            ),
            ScalarModule::new(library_source, library.items, Vec::new()),
        ],
        vec![main_source.clone()],
    ));
    assert_eq!(
        validation.diagnostics.len(),
        1,
        "{:?}",
        validation.diagnostics
    );
    let diagnostic = &validation.diagnostics[0];
    assert_eq!(diagnostic.code, "B0004");
    assert_eq!(
        diagnostic.message,
        "generic argument arity does not match callable declaration"
    );
    assert_eq!(diagnostic.labels[0].span.source, main_source);
    let start = main_text.find("library.read<u32>()").expect("extern call") as u32;
    assert_eq!(
        diagnostic.labels[0].span.range,
        ByteSpan::new(start, start + 19)
    );
}

#[test]
fn derives_all_generic_extern_functions_in_declaration_order() {
    let result = validate_text(
            "%%start\nenv = extern wasm \"helper\" { unsafe i32(i32) read; unit() flush; };\ni32 value = unsafe { env.read(7) };\nenv.flush();\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Extern(extern_decl) = &result.program.items[0] else {
        panic!("extern item");
    };
    assert_eq!(extern_decl.functions.len(), 2);
    assert_eq!(extern_decl.functions[0].name, "read");
    assert!(extern_decl.functions[0].unsafe_marker);
    assert_eq!(extern_decl.functions[1].name, "flush");
    assert!(!extern_decl.functions[1].unsafe_marker);
}

#[test]
fn ordinary_unsafe_extern_requires_structured_unsafe_block() {
    let safe_text = "%%start\nraw = extern wasm \"env\" { unsafe i32(i32) read; };\ni32 value = raw.read(1);\n%%end";
    let safe = validate_text(safe_text);
    let diagnostic = safe
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0012")
        .expect("unsafe-call diagnostic");
    assert_eq!(
        diagnostic.message,
        "unsafe extern call requires an unsafe block"
    );
    assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(71, 82));

    let unsafe_text = "%%start\nraw = extern wasm \"env\" { unsafe i32(i32) read; };\ni32 value = unsafe { raw.read(1) };\n%%end";
    let accepted = validate_text(unsafe_text);
    assert!(
        accepted.diagnostics.is_empty(),
        "{:?}",
        accepted.diagnostics
    );
    let ScalarItem::Binding(binding) = &accepted.program.items[1] else {
        panic!("binding item");
    };
    let ScalarExpression::Block(block) = &binding.value else {
        panic!("unsafe block expression");
    };
    assert!(block.unsafe_context);
}

#[test]
fn derives_valid_scalar_bindings_arithmetic_call_and_if() {
    let result = validate_text(
            "%%start\ni32(i32, i32) add = fn(left, right) {\n\tleft + right\n};\ni32 seed = 20;\ni32 increment = 22;\nbool choose_sum = true;\ni32 result = if (choose_sum) {\n\tadd(seed, increment)\n} else {\n\t0\n};\n%%end",
        );
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    assert_eq!(result.program.items.len(), 5);
    assert!(matches!(result.program.items[0], ScalarItem::Function(_)));
    assert!(matches!(result.program.items[4], ScalarItem::Binding(_)));
}

#[test]
fn reports_integer_range_at_literal_span_and_preserves_i32_max() {
    let invalid_text = "%%start\ni32 value = 2147483648;\n%%end";
    let invalid = validate_text(invalid_text);
    let diagnostic = invalid
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0010")
        .expect("integer range diagnostic");
    let start = invalid_text.find("2147483648").expect("literal") as u32;
    assert_eq!(
        diagnostic.labels[0].span.range,
        ByteSpan::new(start, start + 10)
    );

    let valid = validate_text("%%start\ni32 minimum = 0;\ni32 maximum = 2147483647;\n%%end");
    assert!(
        valid.diagnostics.is_empty(),
        "diagnostics: {:?}",
        valid.diagnostics
    );

    let huge = validate_text(
            "%%start\ni32 value = 999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999;\n%%end",
        );
    assert!(huge.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "B0010"
            && diagnostic.message == "integer literal is outside the resolved target type range"
    }));
}

#[test]
fn project_validation_reports_integer_range_at_literal_span() {
    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), "%%start\ni32 value = 2147483648;\n%%end");
    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source.clone()],
    ));
    let diagnostic = validation
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0010")
        .expect("project integer range diagnostic");
    assert_eq!(diagnostic.labels[0].span.source, source);
    assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(20, 30));
}

#[test]
fn terminated_final_expression_is_unit_in_program_and_project_blocks() {
    let invalid = validate_text("%%start\ni32() f = fn { 1; };\n%%end");
    assert!(invalid
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0003"));
    let ScalarItem::Function(function) = &invalid.program.items[0] else {
        panic!("function item");
    };
    assert_eq!(function.body.terminated_items, vec![true]);

    let valid = validate_text("%%start\ni32() f = fn { 1 };\n%%end");
    assert!(
        valid.diagnostics.is_empty(),
        "diagnostics: {:?}",
        valid.diagnostics
    );
    let ScalarItem::Function(function) = &valid.program.items[0] else {
        panic!("function item");
    };
    assert_eq!(function.body.terminated_items, vec![false]);

    let source = module_source("src/main.w");
    let invalid_project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(
            source.clone(),
            invalid.program.items,
            Vec::new(),
        )],
        vec![source.clone()],
    ));
    assert!(invalid_project
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0003"));

    let valid_project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(
            source.clone(),
            valid.program.items,
            Vec::new(),
        )],
        vec![source],
    ));
    assert!(valid_project.diagnostics.is_empty());
}

#[test]
fn validates_project_callable_parameter_arity_like_single_file_programs() {
    let source = "%%start\ni32(i32, i32) add = fn(value) { value };\n%%end";
    let single_file = validate_text(source);
    assert!(single_file
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0004"));

    let invalid_module_source = module_source("src/math.w");
    let module = module_from_text(invalid_module_source.clone(), source);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(
            invalid_module_source.clone(),
            module.items,
            Vec::new(),
        )],
        vec![invalid_module_source],
    ));
    let project_diagnostic = project
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0004")
        .expect("project callable arity diagnostic");
    assert_eq!(
        project_diagnostic.message,
        single_file
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0004")
            .expect("single-file callable arity diagnostic")
            .message
    );

    let valid_source = "%%start\ni32(i32, i32) add = fn(left, right) { left + right };\n%%end";
    let valid_module_source = module_source("src/math.w");
    let valid_module = module_from_text(valid_module_source.clone(), valid_source);
    let valid_project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(
            valid_module_source.clone(),
            valid_module.items,
            Vec::new(),
        )],
        vec![valid_module_source],
    ));
    assert!(valid_project.diagnostics.is_empty());
}

#[test]
fn rejects_visible_binding_collisions_in_single_and_project_validation() {
    let cases = [
            (
                "%%start\ni32 value = 1;\ni32(i32) f = fn(value) { value };\n%%end",
                "parameter/module",
            ),
            (
                "%%start\ni32(i32) f = fn(outer) { if (true) { i32 outer = 1; outer } else { outer } };\n%%end",
                "nested local/outer",
            ),
            (
                "%%start\ni32(i32, i32) f = fn(value, value) { value };\n%%end",
                "repeated parameter",
            ),
            (
                "%%start\nmath = namespace app \"src/math.w\";\ni32(i32) f = fn(math) { 0 };\n%%end",
                "parameter/namespace",
            ),
        ];
    for (text, case_name) in cases {
        let single = validate_text(text);
        assert!(
            single
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0002"),
            "missing single-program collision diagnostic for {case_name}: {:?}",
            single.diagnostics
        );

        let source = module_source("src/main.w");
        let program = module_from_text(source.clone(), text);
        let namespace_bindings = program
            .items
            .iter()
            .filter_map(|item| match item {
                ScalarItem::Namespace(namespace) => Some(ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target: module_source("src/math.w"),
                    span: namespace.span,
                }),
                _ => None,
            })
            .collect();
        let project = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::new(
                source.clone(),
                program.items,
                namespace_bindings,
            )],
            vec![source],
        ));
        assert!(
            project
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0002"),
            "missing project collision diagnostic for {case_name}: {:?}",
            project.diagnostics
        );
    }

    let valid = validate_text(
            "%%start\ni32 module_value = 1;\ni32(i32) f = fn(parameter) { if (true) { i32 inner = parameter; inner } else { parameter } };\n%%end",
        );
    assert!(
        valid.diagnostics.is_empty(),
        "distinct bindings produced diagnostics: {:?}",
        valid.diagnostics
    );
}

#[test]
fn derives_and_type_checks_top_level_assignment_in_module_scope() {
    let result = validate_text("%%start\ni32 value = 0;\nvalue = 1;\nvalue;\n%%end");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    assert_eq!(result.program.items.len(), 3);
    assert!(matches!(result.program.items[0], ScalarItem::Binding(_)));
    assert!(matches!(
        result.program.items[1],
        ScalarItem::Executable(ScalarBlockItem::Assignment(_))
    ));
    assert!(matches!(
        result.program.items[2],
        ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Name { .. }))
    ));

    let mismatch = validate_text("%%start\nbool value = false;\nvalue = 1;\n%%end");
    assert!(mismatch
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0003"));
    assert_eq!(
        mismatch
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0003")
            .expect("assignment type diagnostic")
            .labels[0]
            .span
            .range,
        ByteSpan::new(28, 37)
    );
}

#[test]
fn validates_unary_nesting_bitwise_types_and_short_circuit_booleans() {
    let result = validate_text(
        "%%start
i32 complemented = ~~~1;
bool inverted = !!!true;
i32 bit_and = 7 & 3;
i32 bit_or = 7 | 3;
i32 bit_xor = 7 ^ 3;
i32 shifted_left = 7 << 1;
i32 shifted_right = 7 >> 1;
bool conjunction = true && false;
bool disjunction = false || true;
%%end",
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

    let ScalarItem::Binding(binding) = &result.program.items[0] else {
        panic!("complement binding");
    };
    assert!(matches!(
        &binding.value,
        ScalarExpression::Unary {
            operator: UnaryOperator::BitwiseNot,
            operand,
            ..
        } if matches!(operand.as_ref(), ScalarExpression::Unary { .. })
    ));
    let ScalarItem::Binding(binding) = &result.program.items[1] else {
        panic!("inversion binding");
    };
    assert!(matches!(
        &binding.value,
        ScalarExpression::Unary {
            operator: UnaryOperator::LogicalNot,
            operand,
            ..
        } if matches!(operand.as_ref(), ScalarExpression::Unary { .. })
    ));
}

#[test]
fn derives_structural_negation_and_contextually_validates_integer_ranges() {
    let text = "%%start\ni32 minimum = -2147483648;\ni32 quotient = -7 / 3;\ni32 remainder = -7 % 3;\n%%end";
    let result = validate_text(text);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

    let ScalarItem::Binding(binding) = &result.program.items[0] else {
        panic!("minimum binding");
    };
    assert!(matches!(
        &binding.value,
        ScalarExpression::Unary {
            operator: UnaryOperator::Negate,
            span,
            operand,
        } if *span == ByteSpan::new(22, 33)
            && matches!(operand.as_ref(), ScalarExpression::Integer { value, .. } if value == &BigInt::from(2147483648u32))
    ));

    for name in ["quotient", "remainder"] {
        let ScalarItem::Binding(binding) = result
            .program
            .items
            .iter()
            .find(|item| matches!(item, ScalarItem::Binding(binding) if binding.name == name))
            .expect("arithmetic binding")
        else {
            panic!("arithmetic binding");
        };
        assert!(matches!(binding.declared_type, ScalarType::I32));
        assert!(matches!(binding.value, ScalarExpression::Binary { .. }));
    }

    let invalid = validate_text("%%start\ni32 value = -2147483649;\n%%end");
    let diagnostic = invalid
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0010")
        .expect("negative range diagnostic");
    assert_eq!(
        diagnostic.message,
        "integer literal is outside the resolved target type range"
    );
    assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(20, 31));

    let unsigned = validate_text("%%start\nu32 value = -1;\n%%end");
    assert!(unsigned.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "B0010"
            && diagnostic.message == "integer literal is outside the resolved target type range"
            && diagnostic.labels[0].span.range == ByteSpan::new(20, 22)
    }));
}

#[test]
fn rejects_invalid_integer_boolean_and_equal_type_operands_at_expression_spans() {
    let text = "%%start
i32 signed = 1;
u32 unsigned = 1;
i32 mismatch = signed & unsigned;
i32 bool_bitwise = true | false;
i32 bool_complement = ~true;
bool integer_inversion = !1;
%%end";
    let result = validate_text(text);
    let diagnostics: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "B0003")
        .collect();
    assert!(diagnostics.len() >= 4, "{:?}", result.diagnostics);
    for expression in ["signed & unsigned", "true | false", "~true", "!1"] {
        let start = text.find(expression).expect("expression") as u32;
        let end = start + expression.len() as u32;
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.labels[0].span.range == ByteSpan::new(start, end)),
            "missing diagnostic span for {expression}: {:?}",
            diagnostics
        );
    }
}

#[test]
fn preserves_scalar_precedence_and_left_associativity() {
    let result = validate_text(
        "%%start\ni32 value = 1 | 2 ^ 3 & 4 << 1 + 2 * 3;\ni32 shifts = 8 >> 1 >> 1;\n%%end",
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(binding) = &result.program.items[0] else {
        panic!("value binding");
    };
    let ScalarExpression::Binary {
        operator,
        left,
        right,
        ..
    } = &binding.value
    else {
        panic!("outer binary expression");
    };
    assert_eq!(*operator, BinaryOperator::BitOr);
    assert!(
        matches!(
            right.as_ref(),
            ScalarExpression::Binary {
                operator: BinaryOperator::BitXor,
                ..
            }
        ),
        "{binding:?}"
    );
    assert!(matches!(left.as_ref(), ScalarExpression::Integer { .. }));
    let ScalarItem::Binding(binding) = &result.program.items[1] else {
        panic!("shift binding");
    };
    assert!(matches!(
        &binding.value,
        ScalarExpression::Binary {
            operator: BinaryOperator::ShiftRight,
            left,
            ..
        } if matches!(left.as_ref(), ScalarExpression::Binary {
            operator: BinaryOperator::ShiftRight,
            ..
        })
    ));
}

#[test]
fn derives_and_validates_ordered_local_mutation_and_unit_while() {
    let result = validate_text(
            "%%start\ni32(i32) loop = fn(start) {\n\ti32 value = start;\n\twhile (value < 3) {\n\t\tvalue = value + 1;\n\t}\n\tvalue\n};\n%%end",
        );
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    let ScalarItem::Function(function) = &result.program.items[0] else {
        panic!("function item");
    };
    assert_eq!(result.program.source, source());
    assert_eq!(function.body.items.len(), 3);
    assert!(matches!(
        function.body.items[0],
        ScalarBlockItem::LocalBinding(_)
    ));
    let ScalarBlockItem::While(while_expression) = &function.body.items[1] else {
        panic!("while item");
    };
    assert_eq!(while_expression.span, ByteSpan::new(57, 100));
    assert!(matches!(
        while_expression.body.items[0],
        ScalarBlockItem::Assignment(_)
    ));
    assert_eq!(
        block_type(
            &while_expression.body,
            &BTreeMap::new(),
            &BTreeSet::new(),
            &BTreeMap::new(),
            &result.program,
            &mut Vec::new(),
            false,
        ),
        ScalarType::Unit
    );
}

#[test]
fn infers_assignment_targets_and_rejects_mismatched_assignment_type() {
    let inferred =
        validate_text("%%start\ni32(i32) f = fn(value) { inferred = value; inferred };\n%%end");
    assert!(
        inferred.diagnostics.is_empty(),
        "{:?}",
        inferred.diagnostics
    );

    let mismatch = validate_text(
        "%%start\ni32(i32) f = fn(value) { i32 local = value; local = true; local };\n%%end",
    );
    assert!(mismatch
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0003"));
}

#[test]
fn validates_ordered_assignment_outputs_and_inferred_project_targets() {
    let valid = validate_text(
            "%%start\n(i32, bool)() pair = fn { 1 };\ni32() result = fn { first, second = pair(); first };\n%%end",
        );
    assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
    let ScalarItem::Function(result) = &valid.program.items[1] else {
        panic!("ordered assignment")
    };
    let ScalarBlockItem::Assignment(assignment) = &result.body.items[0] else {
        panic!("ordered assignment")
    };
    assert_eq!(
        assignment
            .targets
            .iter()
            .map(|target| &target.target)
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );

    for text in [
            "%%start\n(i32, bool)() pair = fn { 1 };\nfirst, second, third = pair();\n%%end",
            "%%start\n(i32, bool)() pair = fn { 1 };\nbool first = false;\ni32 second = 0;\nfirst, second = pair();\n%%end",
            "%%start\ni32 value = 0;\nvalue, value = 1;\n%%end",
        ] {
            let invalid = validate_text(text);
            assert!(
                invalid.diagnostics.iter().any(|diagnostic| {
                    matches!(diagnostic.code.as_str(), "B0002" | "B0003" | "B0004")
                }),
                "missing assignment diagnostic: {:?}",
                invalid.diagnostics
            );
        }

    let child_source = module_source("src/child.w");
    let child = module_from_text(
        child_source.clone(),
        "%%start\ni32() value = fn { 1 };\n%%end",
    );
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\ni32() result = fn { inferred = child.value(); 0 };\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let project = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source,
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "child".to_owned(),
                    target: child_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(child_source, child.items, Vec::new()),
        ],
        Vec::new(),
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn rejects_const_binding_assignments_at_target_span_in_single_file() {
    let cases = [
            (
                "%%start\ni32 MAX_VALUE = 1;\nMAX_VALUE = 2;\n%%end",
                "MAX_VALUE",
            ),
            (
                "%%start\ni32(i32) f = fn(PARAMETER) { PARAMETER = 1; PARAMETER };\n%%end",
                "PARAMETER",
            ),
            (
                "%%start\ni32(i32) F = fn(value) { i32 LOCAL_VALUE = value; LOCAL_VALUE = value; LOCAL_VALUE };\n%%end",
                "LOCAL_VALUE",
            ),
        ];
    for (text, target) in cases {
        let result = validate_text(text);
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0007")
            .expect("const assignment diagnostic");
        let start = text
            .match_indices(target)
            .nth(1)
            .map(|(start, _)| start)
            .expect("assignment target") as u32;
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + target.len() as u32)
        );
    }

    let valid = validate_text("%%start\ni32 MAX_VALUE = 1;\nMAX_VALUE;\n%%end");
    assert!(
        valid.diagnostics.is_empty(),
        "diagnostics: {:?}",
        valid.diagnostics
    );
}

#[test]
fn rejects_const_binding_assignments_at_target_span_in_project() {
    let child_source = module_source("src/child.w");
    let child = module_from_text(child_source.clone(), "%%start\ni32 MAX_VALUE = 1;\n%%end");
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\ni32 value = child.MAX_VALUE;\nchild.MAX_VALUE = value;\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let assignment_target = match &main.items[2] {
        ScalarItem::Executable(ScalarBlockItem::Assignment(assignment)) => assignment.target_span,
        _ => panic!("assignment item"),
    };
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "child".to_owned(),
                    target: child_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(child_source, child.items, Vec::new()),
        ],
        Vec::new(),
    ));
    let diagnostic = validation
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0007")
        .expect("project const assignment diagnostic");
    assert_eq!(diagnostic.labels[0].span.source, main_source);
    assert_eq!(diagnostic.labels[0].span.range, assignment_target);
}

#[test]
fn rejects_non_boolean_while_condition() {
    let result =
        validate_text("%%start\ni32(i32) f = fn(value) { while (value) { value } 0 };\n%%end");
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0005"));
}

#[test]
fn rejects_unknown_name() {
    let result = validate_text("%%start\ni32 value = missing;\n%%end");
    assert_eq!(result.diagnostics[0].code, "B0001");
    assert_eq!(
        result.diagnostics[0].labels[0].span.range,
        ByteSpan::new(20, 27)
    );
}

#[test]
fn unknown_binding_suppresses_only_its_derived_type_diagnostic() {
    let text = "%%start\ni32 value = missing;\nbool sibling = 1;\n%%end";
    let single = validate_text(text);
    assert_eq!(
        single
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0001")
            .count(),
        1
    );
    assert_eq!(
        single
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0003")
            .count(),
        1
    );

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert_eq!(
        project
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0001")
            .count(),
        1
    );
    assert_eq!(
        project
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0003")
            .count(),
        1
    );
}

#[test]
fn unknown_namespace_receiver_suppresses_only_its_derived_type_diagnostic() {
    let text = "%%start\ni32 value = missing.value;\nbool sibling = 1;\n%%end";
    let single = validate_text(text);
    assert_eq!(
        single
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0001")
            .count(),
        1
    );
    assert_eq!(
        single
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0003")
            .count(),
        1
    );

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert_eq!(
        project
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0001")
            .count(),
        1
    );
    assert_eq!(
        project
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0003")
            .count(),
        1
    );
}

#[test]
fn unknown_callable_suppresses_only_its_derived_type_diagnostic() {
    let text = "%%start\ni32 value = missing();\nbool sibling = 1;\n%%end";
    let single = validate_text(text);
    assert_eq!(
        single
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0001")
            .count(),
        1
    );
    assert_eq!(
        single
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0003")
            .count(),
        1
    );

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert_eq!(
        project
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0001")
            .count(),
        1
    );
    assert_eq!(
        project
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0003")
            .count(),
        1
    );
}

#[test]
fn rejects_invalid_type() {
    let result = validate_text("%%start\ni32 value = 1;\nu32 other = value;\n%%end");
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0003"));
}

#[test]
fn rejects_invalid_call_arity() {
    let result = validate_text(
        "%%start\ni32(i32) one = fn(value) { value };\ni32 result = one(1, 2);\n%%end",
    );
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0004"));
}

#[test]
fn rejects_non_boolean_condition_and_mismatched_branches() {
    let condition = validate_text("%%start\ni32 value = if (1) { 1 } else { 1 };\n%%end");
    assert!(condition
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0005"));
    let branches = validate_text("%%start\ni32 value = if (true) { 1 } else { true };\n%%end");
    assert!(branches.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "B0006"
            && diagnostic.message == "conditional branches must have equal types"
    }));
}

#[test]
fn derives_namespace_and_qualified_member_without_text_reparsing() {
    let result = validate_text(
        "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.add(20, 22);\n%%end",
    );
    let ScalarItem::Namespace(namespace) = &result.program.items[0] else {
        panic!("namespace item");
    };
    assert_eq!(namespace.binding, "math");
    assert_eq!(namespace.package, "app");
    assert_eq!(namespace.path, "\"src/math.w\"");
    assert_eq!(namespace.binding_span, ByteSpan::new(8, 12));
    assert_eq!(namespace.package_span, ByteSpan::new(25, 28));
    assert_eq!(namespace.path_span, ByteSpan::new(29, 41));

    let ScalarItem::Binding(binding) = &result.program.items[1] else {
        panic!("result binding");
    };
    let ScalarExpression::Call {
        receiver,
        name,
        receiver_span,
        name_span,
        ..
    } = &binding.value
    else {
        panic!("qualified call");
    };
    assert_eq!(receiver.as_deref(), Some("math"));
    assert_eq!(name, "add");
    assert_eq!(*receiver_span, Some(ByteSpan::new(56, 60)));
    assert_eq!(*name_span, ByteSpan::new(61, 64));
}

#[test]
fn derives_generic_call_type_arguments_with_unresolved_type_and_span() {
    let result = validate_text("%%start\nu32 result = core.wrap<u32>(value, \"exact\");\n%%end");
    let ScalarItem::Binding(binding) = &result.program.items[0] else {
        panic!("result binding");
    };
    let ScalarExpression::Call {
        receiver,
        name,
        type_arguments,
        arguments,
        ..
    } = &binding.value
    else {
        panic!("generic call");
    };
    assert_eq!(receiver.as_deref(), Some("core"));
    assert_eq!(name, "wrap");
    assert_eq!(arguments.len(), 2);
    assert_eq!(type_arguments.len(), 1);
    assert_eq!(type_arguments[0].ty, ScalarType::U32);
    assert_eq!(type_arguments[0].span, ByteSpan::new(31, 34));
    let serialized = serde_json::to_string(type_arguments).expect("serialize type arguments");
    let restored: Vec<ScalarTypeArgument> =
        serde_json::from_str(&serialized).expect("deserialize type arguments");
    assert_eq!(restored, *type_arguments);
}

#[test]
fn derives_u32_literal_context_for_generic_integer_extension() {
    let text = "%%start\nu64 result = core.int_extend<u64>(4294967295);\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let ScalarItem::Binding(binding) = &single.program.items[0] else {
        panic!("extension binding");
    };
    assert_eq!(binding.declared_type, ScalarType::U64);

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn resolves_member_expression_sources_for_generic_integer_conversions() {
    let text = "%%start\nstruct utf8 {\n\tu64 length;\n}\nu32(utf8) truncate = fn(text) { core.int_trunc<u32>(text.length) };\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn validates_generic_integer_conversions_in_single_file_and_project() {
    let text = "%%start\ni8 signed_eight = 1;\ni16 signed_sixteen = core.int_extend<i16>(signed_eight);\ni32 signed_thirty_two = core.int_extend<i32>(signed_sixteen);\ni64 signed_sixty_four = core.int_extend<i64>(signed_thirty_two);\ni128 signed_full = core.int_extend<i128>(signed_sixty_four);\nu8 unsigned_eight = core.int_trunc<u8>(signed_full);\nu16 unsigned_sixteen = core.int_extend<u16>(unsigned_eight);\nu32 unsigned_thirty_two = core.int_extend<u32>(unsigned_sixteen);\nu64 unsigned_sixty_four = core.int_extend<u64>(unsigned_thirty_two);\nu128 unsigned_full = core.int_extend<u128>(unsigned_sixty_four);\nu8 narrow_unsigned = core.int_trunc<u8>(unsigned_full);\ni8 narrow_signed = core.int_trunc<i8>(unsigned_full);\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

    for text in [
        "%%start\nu32 source = 1;\nu32 value = core.int_trunc<u32>(source);\n%%end",
        "%%start\nu32 source = 1;\nu64 value = core.int_trunc<u64>(source);\n%%end",
        "%%start\nu64 source = 1;\nu32 value = core.int_extend<u32>(source);\n%%end",
        "%%start\nu32 source = 1;\nu32 value = core.int_extend<u32>(source);\n%%end",
        "%%start\nbool source = true;\nu32 value = core.int_extend<u32>(source);\n%%end",
        "%%start\nu64 source = 1;\nu32 value = core.int_trunc<bool>(source);\n%%end",
        "%%start\nu64 source = 1;\nu32 value = core.int_trunc(source);\n%%end",
        "%%start\nu64 source = 1;\nu32 value = core.int_trunc<u32, u16>(source);\n%%end",
        "%%start\nu64 source = 1;\nu32 value = core.int_trunc<u32>();\n%%end",
        "%%start\nu64 source = 1;\nu32 value = core.int_trunc<u32>(source, source);\n%%end",
    ] {
        let invalid = validate_text(text);
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003" || diagnostic.code == "B0004"),
            "{text}: {:?}",
            invalid.diagnostics
        );
    }
}

#[test]
fn validates_float_conversions_in_single_file_and_project() {
    let text = "%%start\nu64 uint_source = 42;\ni64 sint_source = 42;\nf64 wide_source = 1.5;\nf32 narrow_source = 1.5;\nf64 from_uint = core.uint_to_float<f64>(uint_source);\nf32 from_sint = core.sint_to_float<f32>(sint_source);\nf64 from_uint_literal = core.uint_to_float<f64>(42);\nf32 from_sint_literal = core.sint_to_float<f32>(-3);\ni32 to_sint = core.float_to_sint_trunc<i32>(wide_source);\nu64 to_uint = core.float_to_uint_trunc<u64>(narrow_source);\ni32 to_sint_literal = core.float_to_sint_trunc<i32>(1.5);\nf32 narrowed = core.float_trunc<f32>(wide_source);\nf32 narrowed_literal = core.float_trunc<f32>(1.5);\nf64 widened = core.float_extend<f64>(narrow_source);\nf64 widened_literal = core.float_extend<f64>(1.5);\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

    for (text, code) in [
        (
            "%%start\nu64 source = 1;\nu32 value = core.uint_to_float<u32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\ni64 source = 1;\nf64 value = core.sint_to_float<i64>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf64 source = 1.5;\nf64 value = core.float_to_sint_trunc<f64>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf32 source = 1.5;\nf32 value = core.float_to_uint_trunc<f32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf64 source = 1.5;\nu32 value = core.float_trunc<u32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf32 source = 1.5;\ni32 value = core.float_extend<i32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\ni64 source = 1;\nf64 value = core.uint_to_float<f64>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nu64 source = 1;\nf32 value = core.sint_to_float<f32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf64 value = core.uint_to_float<f64>(-1);\n%%end",
            "B0003",
        ),
        (
            "%%start\ni64 source = 1;\ni32 value = core.float_to_sint_trunc<i32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nu64 source = 1;\nf32 value = core.float_trunc<f32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf32 source = 1.5;\nu32 value = core.float_to_sint_trunc<u32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf32 source = 1.5;\ni32 value = core.float_to_uint_trunc<i32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf32 source = 1.5;\nf32 value = core.float_trunc<f32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf64 source = 1.5;\nf64 value = core.float_extend<f64>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf64 source = 1.5;\nf64 value = core.float_trunc<f64>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nf32 source = 1.5;\nf32 value = core.float_extend<f32>(source);\n%%end",
            "B0003",
        ),
        (
            "%%start\nu64 source = 1;\nf64 value = core.uint_to_float(source);\n%%end",
            "B0004",
        ),
        (
            "%%start\nu64 source = 1;\nf64 value = core.uint_to_float<f64, f32>(source);\n%%end",
            "B0004",
        ),
        (
            "%%start\nu64 source = 1;\nf64 value = core.uint_to_float<f64>();\n%%end",
            "B0004",
        ),
        (
            "%%start\nu64 source = 1;\nf64 value = core.sint_to_float<f64>(source, source);\n%%end",
            "B0004",
        ),
    ] {
        let invalid = validate_text(text);
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "{text}: {:?}",
            invalid.diagnostics
        );
    }
}

#[test]
fn validates_bitcast_in_single_file_and_project() {
    let text = "%%start\nchar letter = 'A';\nu32 code = core.bitcast<u32>(letter);\nf32 bits = core.bitcast<f32>(code);\nu32 roundtrip = core.bitcast<u32>(bits);\ni32 signed = core.bitcast<i32>(roundtrip);\nf64 wide = 1.5;\nu64 wbits = core.bitcast<u64>(wide);\nf64 back = core.bitcast<f64>(wbits);\ni64 sbits = core.bitcast<i64>(wide);\nbool flag = true;\nbool same = core.bitcast<bool>(flag);\nu8 small = 7;\ni8 tiny = core.bitcast<i8>(small);\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

    for (text, code) in [
            (
                "%%start\nu32 source = 1;\nu64 value = core.bitcast<u64>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nu32 source = 1;\nu16 value = core.bitcast<u16>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nchar letter = 'A';\nu64 value = core.bitcast<u64>(letter);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf32 narrow = 1.5;\nf64 value = core.bitcast<f64>(narrow);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf64 wide = 1.5;\nf32 value = core.bitcast<f32>(wide);\n%%end",
                "B0003",
            ),
            (
                "%%start\nf32 narrow = 1.5;\ni64 value = core.bitcast<i64>(narrow);\n%%end",
                "B0003",
            ),
            (
                "%%start\nu32 source = 1;\n*?u8 value = core.bitcast<*?u8>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\n*?u8 storage = null;\nu64 value = core.bitcast<u64>(storage);\n%%end",
                "B0003",
            ),
            (
                "%%start\nstruct Pair {\n\ti32 x;\n\ti32 y;\n}\nPair holder = { .x = 1; .y = 2; };\nu64 value = core.bitcast<u64>(holder);\n%%end",
                "B0003",
            ),
            (
                "%%start\nstruct Pair {\n\ti32 x;\n\ti32 y;\n}\nu32 source = 1;\nPair value = core.bitcast<Pair>(source);\n%%end",
                "B0003",
            ),
            (
                "%%start\nu32 source = 1;\nu32 value = core.bitcast(source);\n%%end",
                "B0004",
            ),
            (
                "%%start\nu32 source = 1;\nu32 value = core.bitcast<u32, u32>(source);\n%%end",
                "B0004",
            ),
            (
                "%%start\nchar letter = 'A';\nu32 value = core.bitcast<u32>();\n%%end",
                "B0004",
            ),
            (
                "%%start\nchar letter = 'A';\nu32 value = core.bitcast<u32>(letter, letter);\n%%end",
                "B0004",
            ),
        ] {
            let invalid = validate_text(text);
            assert!(
                invalid
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "{text}: {:?}",
                invalid.diagnostics
            );
        }
}

#[test]
fn validates_raw_offset_and_load_under_unsafe_in_single_file_and_project() {
    let text = "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset<u8>(pointer, index)) } };\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

    let missing_unsafe = validate_text(
            "%%start\nu8(*?u8, i64) read = fn(pointer, index) { core.load<u8>(core.offset<u8>(pointer, index)) };\n%%end",
        );
    assert!(
        missing_unsafe
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0012"),
        "{:?}",
        missing_unsafe.diagnostics
    );

    for (text, code) in [
            (
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset<u8>(pointer)) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset<u8>(pointer, index, index)) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset(pointer, index)) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u8>(pointer, index) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\nu8(*?u8, u64) read = fn(pointer, index) { unsafe { core.load<u8>(core.offset<u8>(pointer, index)) } };\n%%end",
                "B0003",
            ),
            (
                "%%start\nu8(u8, i64) read = fn(value, index) { unsafe { core.load<u8>(core.offset<u8>(value, index)) } };\n%%end",
                "B0003",
            ),
            (
                "%%start\nu8(*?u8, i64) read = fn(pointer, index) { unsafe { core.load<u16>(core.offset<u8>(pointer, index)) } };\n%%end",
                "B0003",
            ),
            (
                "%%start\nunit(*?u8) consume = fn(pointer) { unsafe { core.load<unit>(pointer) } };\n%%end",
                "B0003",
            ),
        ] {
            let invalid = validate_text(text);
            assert!(
                invalid
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "{text}: {:?}",
                invalid.diagnostics
            );
        }
}

#[test]
fn validates_pointer_cast_under_unsafe_in_single_file_and_project() {
    let text = "%%start\nstruct Block {\n\tu8 tag;\n\tu32 value;\n}\n*?u8 base = null;\n*?u8 roundtrip = unsafe { core.pointer_cast<*?u8>(core.pointer_cast<*?Block>(base)) };\n*?u8(*?u8) ident = fn(pointer) { unsafe { core.pointer_cast<*?u8>(pointer) } };\nu8[4] bytes = [1, 2, 3, 4];\n*?u8 addr = unsafe { core.pointer_cast<*?u8>(&?bytes[2]) };\n*u8 shared = unsafe { core.pointer_cast<*u8>(base) };\n*!u8 exclusive = unsafe { core.pointer_cast<*!u8>(base) };\n*?u8 nullraw = unsafe { core.pointer_cast<*?u8>(null) };\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

    let missing_unsafe = validate_text(
        "%%start\n*?u8(*?u8) cast = fn(pointer) { core.pointer_cast<*?u8>(pointer) };\n%%end",
    );
    assert!(
        missing_unsafe
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0012"),
        "{:?}",
        missing_unsafe.diagnostics
    );
    let missing_unsafe_source = module_source("src/main.w");
    let missing_unsafe_program = module_from_text(
        missing_unsafe_source.clone(),
        "%%start\n*?u8(*?u8) cast = fn(pointer) { core.pointer_cast<*?u8>(pointer) };\n%%end",
    );
    let missing_unsafe_project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(
            missing_unsafe_source.clone(),
            missing_unsafe_program.items,
            Vec::new(),
        )],
        vec![missing_unsafe_source],
    ));
    assert!(
        missing_unsafe_project
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0012"),
        "{:?}",
        missing_unsafe_project.diagnostics
    );

    for (text, code) in [
            (
                "%%start\n*?u8(*?u8) cast = fn(pointer) { unsafe { core.pointer_cast(pointer) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\n*?u8(*?u8) cast = fn(pointer) { unsafe { core.pointer_cast<*?u8, *?u8>(pointer) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\n*?u8(*?u8) cast = fn(pointer) { unsafe { core.pointer_cast<*?u8>() } };\n%%end",
                "B0004",
            ),
            (
                "%%start\n*?u8(*?u8) cast = fn(pointer) { unsafe { core.pointer_cast<*?u8>(pointer, pointer) } };\n%%end",
                "B0004",
            ),
            (
                "%%start\nu32(*?u8) cast = fn(pointer) { unsafe { core.pointer_cast<u32>(pointer) } };\n%%end",
                "B0003",
            ),
            (
                "%%start\nstruct Pair {\n\ti32 x;\n\ti32 y;\n}\nPair(*?u8) cast = fn(pointer) { unsafe { core.pointer_cast<Pair>(pointer) } };\n%%end",
                "B0003",
            ),
            (
                "%%start\n*?u8(u8) cast = fn(value) { unsafe { core.pointer_cast<*?u8>(value) } };\n%%end",
                "B0003",
            ),
            (
                "%%start\n*?u8(*u8) cast = fn(reader) { unsafe { core.pointer_cast<*?u8>(reader) } };\n%%end",
                "B0003",
            ),
        ] {
            let single = validate_text(text);
            assert!(
                single
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "{text}: {:?}",
                single.diagnostics
            );
            let source = module_source("src/main.w");
            let program = module_from_text(source.clone(), text);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
                vec![source],
            ));
            assert!(
                project
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "{text}: {:?}",
                project.diagnostics
            );
        }
}

#[test]
fn checked_pointer_cast_preserves_deeply_nested_cfg_construction() {
    let depth = 8;
    let text = format!(
            "%%start\nunit(*?u8) nested = fn(raw) {{ {}*!u8 checked = unsafe {{ core.pointer_cast<*!u8>(raw) }}; *checked = 1; {} }};\n%%end",
            "unsafe { ".repeat(depth),
            " };".repeat(depth)
        );
    let (single, project) = both_reference_functions(&text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn checked_pointer_cast_records_source_owner_and_loan() {
    let text =
        "%%start\n*!u8(*?u8) cast = fn(raw) { unsafe { core.pointer_cast<*!u8>(raw) } };\n%%end";
    let (single, project) = both_reference_functions(text);
    for (items, diagnostics) in [
        (&single.program.items, &single.diagnostics),
        (&project.project.modules[0].items, &project.diagnostics),
    ] {
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ScalarItem::Function(function) = &items[0] else {
            panic!("function")
        };
        let cast = function
            .reference_cfg
            .points
            .iter()
            .find(|point| {
                matches!(
                    point.kind,
                    CfgPointKind::Call {
                        target: ResolvedCallTarget::Core(CoreOperationId::PointerCast),
                        ..
                    }
                )
            })
            .expect("cast call");
        let ReferenceOrigin::Fresh { allocation, loan } = sole_fact(function, cast) else {
            panic!("checked cast origin")
        };
        assert_eq!(
            allocation.declaration_span.start,
            text.find("raw)").unwrap() as u32
        );
        assert_eq!(
            loan.origin_place.binding.declaration_span,
            allocation.declaration_span
        );
        assert_eq!(
            loan.creation_span.start,
            text.find("core.pointer_cast").unwrap() as u32
        );
        assert_eq!(loan.mode, ScalarReferenceMutability::Mutable);
    }
}

#[test]
fn validates_direct_pointer_cast_dereference_in_single_file_and_project() {
    for destination in ["*u8", "*!u8"] {
        for (text, expected_code) in [
                (
                    format!(
                        "%%start\nu8(*?u8) read = fn(pointer) {{ unsafe {{ *core.pointer_cast<{destination}>(pointer) }} }};\n%%end"
                    ),
                    None,
                ),
                (
                    format!(
                        "%%start\nu8(*?u8) read = fn(pointer) {{ *core.pointer_cast<{destination}>(pointer) }};\n%%end"
                    ),
                    Some("B0012"),
                ),
            ] {
                let single = validate_text(&text);
                let source = module_source("src/main.w");
                let program = module_from_text(source.clone(), &text);
                let project = validate_scalar_project(ScalarProject::new(
                    vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
                    vec![source],
                ));
                if let Some(code) = expected_code {
                    assert!(
                        single
                            .diagnostics
                            .iter()
                            .any(|diagnostic| diagnostic.code == code),
                        "{text}: {:?}",
                        single.diagnostics
                    );
                    assert!(
                        project
                            .diagnostics
                            .iter()
                            .any(|diagnostic| diagnostic.code == code),
                        "{text}: {:?}",
                        project.diagnostics
                    );
                } else {
                    assert!(
                        single.diagnostics.is_empty(),
                        "{text}: {:?}",
                        single.diagnostics
                    );
                    assert!(
                        project.diagnostics.is_empty(),
                        "{text}: {:?}",
                        project.diagnostics
                    );
                }
            }
    }

    for destination in ["*Record", "*!Record"] {
        for (body, expected_code) in [
            (
                format!("unsafe {{ (*core.pointer_cast<{destination}>(raw)).tag }}"),
                None,
            ),
            (
                format!("(*core.pointer_cast<{destination}>(raw)).tag"),
                Some("B0012"),
            ),
        ] {
            let text = format!(
                    "%%start\nstruct Record {{ u8 tag; }}\nu8(*?u8) read = fn(raw) {{ {body} }};\n%%end"
                );
            let single = validate_text(&text);
            let source = module_source("src/main.w");
            let program = module_from_text(source.clone(), &text);
            let project = validate_scalar_project(ScalarProject::new(
                vec![ScalarModule::from_program(program, Vec::new())],
                vec![source],
            ));
            if let Some(code) = expected_code {
                assert!(
                    single
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == code),
                    "{text}: {:?}",
                    single.diagnostics
                );
                assert!(
                    project
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == code),
                    "{text}: {:?}",
                    project.diagnostics
                );
            } else {
                assert!(
                    single.diagnostics.is_empty(),
                    "{text}: {:?}",
                    single.diagnostics
                );
                assert!(
                    project.diagnostics.is_empty(),
                    "{text}: {:?}",
                    project.diagnostics
                );
                for (items, structure) in [
                    (&single.program.items, &single.program.structs[0]),
                    (
                        &project.project.modules[0].items,
                        &project.project.modules[0].structs[0],
                    ),
                ] {
                    let ScalarItem::Function(function) = &items[0] else {
                        panic!("read function");
                    };
                    let ScalarBlockItem::Expression(ScalarExpression::Block(block)) =
                        &function.body.items[0]
                    else {
                        panic!("unsafe block");
                    };
                    let ScalarExpression::Dereference {
                        place: ScalarPlace::Field { base, field, .. },
                        ..
                    } = &block.final_output_values[0].value
                    else {
                        panic!("field read");
                    };
                    let ScalarPlace::Dereference { pointer, .. } = base.as_ref() else {
                        panic!("field base");
                    };
                    let ScalarExpression::Call { type_arguments, .. } = pointer.as_ref() else {
                        panic!("cast call");
                    };
                    assert!(
                        matches!(type_arguments[0].ty, ScalarType::CheckedReference { ref inner, .. } if inner.as_ref() == &ScalarType::Struct(structure.id.clone()))
                    );
                    assert_eq!(
                        field,
                        &ScalarFieldReference::Resolved(structure.fields[0].id.clone())
                    );
                }
            }
        }
    }

    let text = "%%start\nstruct Record { u8 tag; }\nu8(*Record) read = fn(pointer) { unsafe { (*pointer).tag } };\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn validates_generic_ordinary_two_output_call_in_order() {
    let result = validate_text(
        "%%start\n(u64, bool)() pair = fn { 1 };\nu64 first, bool second = pair<u32>();\n%%end",
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(binding) = &result.program.items[1] else {
        panic!("generic binding");
    };
    assert_eq!(
        binding.output_origin,
        ScalarBindingOutputOrigin::SingleExpression
    );
    assert_eq!(binding.output_sequence.outputs.len(), 2);
    assert_eq!(binding.output_sequence.outputs[0].ty, ScalarType::U64);
    assert_eq!(binding.output_sequence.outputs[1].ty, ScalarType::Bool);
    assert_eq!(binding.output_values[0].position, 0);
    assert_eq!(binding.output_values[1].position, 1);
    let call_span = expression_span(&binding.value);
    assert_eq!(binding.output_values[0].span, call_span);
    assert_eq!(binding.output_values[1].span, call_span);
}

#[test]
fn derives_and_substitutes_scoped_generic_function_parameters() {
    let result = validate_text(
            "%%start\ngeneric T;\nT(T) identity = fn(value) { value };\ni64 result = identity<i64>(1);\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Function(function) = &result.program.items[0] else {
        panic!("generic function");
    };
    assert_eq!(function.generic_parameters.len(), 1);
    assert_eq!(function.generic_parameters[0].name, "T");
    let ScalarItem::Binding(binding) = &result.program.items[1] else {
        panic!("generic call binding");
    };
    assert_eq!(binding.declared_type, ScalarType::I64);
}

#[test]
fn resolves_ordinary_and_generic_overload_arms_in_single_file_and_project() {
    let text = "%%start\ncast = overload {\n    i32(i64) => fn(value) { value };\n    generic T;\n    T(T) => fn(value) { value };\n};\npair = overload {\n    generic T;\n    (T, T)(T) => fn(value) { value };\n};\ni64 wide = 1;\ni32 narrow = cast(wide);\nbool flag = true;\nbool copied = cast(flag);\ni64 first, i64 second = pair(wide);\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let ScalarItem::Function(overload) = &single.program.items[0] else {
        panic!("overload declaration");
    };
    assert_eq!(overload.overload_arms.len(), 2);

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn resolves_struct_typed_overload_arm_in_single_file_and_project() {
    let text = "%%start\nstruct utf8 {\n\t*?u8 data;\n\tu64 length;\n}\nparse = overload {\n    *u64(utf8) => fn(text) {\n        *u64 result = null;\n        result\n    };\n};\nutf8 input = { .data = null; .length = 0; };\n*u64 parsed = parse(input);\n%%end";
    let single = validate_text(text);
    assert!(
        !single
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"),
        "{:?}",
        single.diagnostics
    );
    assert!(
        !single
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0004"),
        "{:?}",
        single.diagnostics
    );
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program, Vec::new())],
        vec![source],
    ));
    assert!(
        !project
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0003"),
        "{:?}",
        project.diagnostics
    );
    assert!(
        !project
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0004"),
        "{:?}",
        project.diagnostics
    );
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn keeps_generic_overload_arm_placeholders_when_resolving_struct_typed_arms() {
    let text = "%%start\nstruct utf8 {\n\t*?u8 data;\n\tu64 length;\n}\nconvert = overload {\n    u64(utf8) => fn(text) { text.length };\n    generic T;\n    T(T) => fn(value) { value };\n};\nutf8 input = { .data = null; .length = 0; };\nu64 width = convert(input);\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let ScalarItem::Function(overload) = &single.program.items[0] else {
        panic!("overload declaration");
    };
    assert_eq!(overload.overload_arms.len(), 2);
    let ScalarType::Callable { parameters, .. } = &overload.overload_arms[1].signature else {
        panic!("generic arm signature");
    };
    assert!(
        matches!(&parameters[0], ScalarType::Named { name, .. } if name == "T"),
        "{:?}",
        overload.overload_arms[1].signature
    );

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn resolves_qualified_generic_overload_from_value_argument_and_records_selection() {
    let child_source = module_source("src/child.w");
    let child = module_from_text(
            child_source.clone(),
            "%%start\nidentity = overload {\n    i32(i64) => fn(value) { 7 };\n    generic T;\n    T(T) => fn(value) { value };\n};\n%%end",
        );
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\nchar copied = child.identity('g');\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "child".to_owned(),
                    target: child_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![main_source, child_source],
    ));
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(binding) = &result.project.modules[0].items[1] else {
        panic!("qualified overload binding");
    };
    let ScalarExpression::Call {
        overload_selection: Some(selection),
        ..
    } = &binding.value
    else {
        panic!("qualified overload selection");
    };
    assert_eq!(selection.arm_index, 1);
    assert_eq!(selection.substitutions.get("T"), Some(&ScalarType::Char));
}

#[test]
fn keeps_private_overloads_local_and_reports_imported_private_overloads_at_member_tokens() {
    let child_source = module_source("src/child.w");
    let child = module_from_text(
            child_source.clone(),
            "%%start\n_ordinary = overload {\n    i32(i64) => fn(value) { value };\n};\n_identity = overload {\n    i32(i64) => fn(value) { 7 };\n    generic T;\n    T(T) => fn(value) { value };\n};\npublic_ordinary = overload {\n    i32(i64) => fn(value) { value };\n};\npublic_identity = overload {\n    i32(i64) => fn(value) { 7 };\n    generic T;\n    T(T) => fn(value) { value };\n};\ni32 local_ordinary = _ordinary(1);\nchar local_generic = _identity('l');\n%%end",
        );
    let main_source = module_source("src/main.w");
    let main_text = "%%start\nchild = namespace generic_overloads \"src/child.w\";\ni32 public_ordinary = child.public_ordinary(1);\nchar public_value = child.public_identity('p');\ni32 ordinary = child._ordinary(1);\nchar value = child._identity('g');\n%%end";
    let main = module_from_text(main_source.clone(), main_text);
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "child".to_owned(),
                    target: child_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![main_source.clone(), child_source],
    ));
    let diagnostics: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "M0002")
        .collect();
    assert_eq!(
        diagnostics.len(),
        result.diagnostics.len(),
        "{:?}",
        result.diagnostics
    );
    assert_eq!(diagnostics.len(), 2, "{:?}", result.diagnostics);
    for (diagnostic, member) in diagnostics.iter().zip(["_ordinary", "_identity"]) {
        let qualified = format!("child.{member}");
        let start = main_text.find(&qualified).expect("private member") + "child.".len();
        let start = u32::try_from(start).expect("source span");
        let end = start + u32::try_from(member.len()).expect("member span");
        assert_eq!(diagnostic.labels[0].span.source, main_source);
        assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(start, end));
    }
}

#[test]
fn suppresses_derived_assignment_diagnostics_for_private_overload_output_sequences() {
    let child_source = module_source("src/child.w");
    let child = module_from_text(
            child_source.clone(),
            "%%start\n_private_pair = overload {\n    (i32, bool)(i64) => fn(value) { 1, true };\n};\npublic_pair = overload {\n    (i32, bool)(i64) => fn(value) { 1, true };\n};\ni32 local_number, bool local_flag = _private_pair(1);\n%%end",
        );
    let main_source = module_source("src/main.w");
    let main_text = "%%start\nchild = namespace output_sequences \"src/child.w\";\ni32 public_number, bool public_flag = child.public_pair(1);\nnumber, flag = child._private_pair(1);\n%%end";
    let main = module_from_text(main_source.clone(), main_text);
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "child".to_owned(),
                    target: child_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(child_source.clone(), child.items, Vec::new()),
        ],
        vec![main_source.clone(), child_source],
    ));
    assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
    let diagnostic = &result.diagnostics[0];
    assert_eq!(diagnostic.code, "M0002");
    assert_eq!(diagnostic.labels[0].span.source, main_source);
    let start = main_text
        .find("child._private_pair")
        .expect("private overload member")
        + "child.".len();
    let start = u32::try_from(start).expect("source span");
    let end = start + u32::try_from("_private_pair".len()).expect("member span");
    assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(start, end));
    assert!(!result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0003"));
    assert!(!result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0004"));
}

#[test]
fn records_distinct_overload_arms_for_shared_input_types() {
    let result = validate_text(
            "%%start\nselect = overload {\n    (i32, bool)(i64) => fn(value) { 1, true };\n    (bool, i32)(i64) => fn(value) { true, 1 };\n};\ni64 input = 1;\ni32 integer, bool flag = select(input);\nbool boolean, i32 number = select(input);\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(integer) = &result.program.items[2] else {
        panic!("integer overload call");
    };
    let ScalarExpression::Call {
        overload_selection: Some(integer_selection),
        ..
    } = &integer.value
    else {
        panic!("integer overload selection");
    };
    assert_eq!(integer_selection.arm_index, 0);
    let ScalarItem::Binding(boolean) = &result.program.items[3] else {
        panic!("boolean overload call");
    };
    let ScalarExpression::Call {
        overload_selection: Some(boolean_selection),
        ..
    } = &boolean.value
    else {
        panic!("boolean overload selection");
    };
    assert_eq!(boolean_selection.arm_index, 1);
}

#[test]
fn reports_overload_candidate_failures_at_call_spans() {
    let explicit = validate_text(
            "%%start\nselect = overload { i32(i32) => fn(value) { value }; };\ni32 value = select<i32>(1);\n%%end",
        );
    assert!(explicit.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "overload calls do not accept explicit type arguments"
    }));
    let ambiguous = validate_text(
            "%%start\nselect = overload { i32(i32) => fn(value) { value }; i32(i32) => fn(value) { value }; };\ni32 value = select(1);\n%%end",
        );
    assert!(ambiguous
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "overload call is ambiguous"));
}

#[test]
fn infers_integer_literal_overloads_only_from_unique_parameter_or_output_context() {
    let constrained = "%%start\nidentity = overload { generic T; T(T) => fn(value) { value }; };\ni64 value = identity(1);\n%%end";
    let single = validate_text(constrained);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), constrained);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);

    let unconstrained = "%%start\nidentity = overload { generic T; T(T) => fn(value) { value }; };\nidentity(1);\n%%end";
    let single = validate_text(unconstrained);
    assert!(single.diagnostics.iter().any(|diagnostic| {
        diagnostic.message
            == "integer literal requires a unique overload parameter or output context"
    }));

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), unconstrained);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.iter().any(|diagnostic| {
        diagnostic.message
            == "integer literal requires a unique overload parameter or output context"
    }));

    let ambiguous = "%%start\nselect = overload { i32(i32) => fn(value) { value }; i64(i64) => fn(value) { value }; };\nselect(1);\n%%end";
    let single = validate_text(ambiguous);
    assert!(single
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "overload call is ambiguous"));

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), ambiguous);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "overload call is ambiguous"));
}

#[test]
fn substitutes_generic_zero_one_and_multiple_outputs_in_single_file_and_project() {
    let text = "%%start\ngeneric T;\nT(T) identity = fn(value) { value };\ngeneric T;\nunit(T) discard = fn(value) { value; };\ngeneric T;\n(T, T)(T) pair = fn(value) { value };\ni64 one = identity<i64>(1);\ndiscard<i64>(1);\ni64 first, i64 second = pair<i64>(1);\n%%end";
    let single = validate_text(text);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let ScalarItem::Binding(binding) = &single.program.items[5] else {
        panic!("multiple-output binding");
    };
    assert_eq!(binding.output_sequence.outputs[0].ty, ScalarType::I64);
    assert_eq!(binding.output_sequence.outputs[1].ty, ScalarType::I64);

    let source = module_source("src/main.w");
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn rejects_duplicate_namespace_binding_in_single_program() {
    let result = validate_text(
            "%%start\nmath = namespace app \"src/first.w\";\nmath = namespace app \"src/second.w\";\n%%end",
        );
    let diagnostics: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "B0002")
        .collect();
    let ScalarItem::Namespace(second) = &result.program.items[1] else {
        panic!("second namespace item");
    };
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].labels[0].span.range, second.span);
}

#[test]
fn reports_ascii_case_collision_with_first_declaration_label_in_single_file() {
    let result = validate_text("%%start\ni32 value = 1;\ni32 VALUE = 2;\n%%end");
    let first_span = match &result.program.items[0] {
        ScalarItem::Binding(binding) => binding.span,
        _ => panic!("first binding"),
    };
    let second_span = match &result.program.items[1] {
        ScalarItem::Binding(binding) => binding.span,
        _ => panic!("second binding"),
    };
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0008")
        .expect("case collision diagnostic");
    assert_eq!(diagnostic.labels[0].span.range, second_span);
    assert_eq!(
        diagnostic.labels[1].kind,
        crate::DiagnosticLabelKind::Secondary
    );
    assert_eq!(diagnostic.labels[1].span.range, first_span);
}

#[test]
fn reports_case_collisions_for_parameters_and_locals_without_changing_lookup() {
    let result = validate_text(
            "%%start\ni32(i32, i32) parameter_collision = fn(first, FIRST) { first };\ni32(i32) local_collision = fn(value) { i32 VALUE = value; i32 value = value; value };\n%%end",
        );
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0008")
            .count(),
        2
    );
    assert!(!result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0001"));
}

#[test]
fn decodes_all_scalar_string_escapes_and_unicode_values() {
    let result = validate_text(
        r##"%%start
std.utf8 value = "\\\"\'\n\r\t\0\u{0}\u{41}\u{1F600}";
%%end"##,
    );
    let ScalarItem::Binding(binding) = &result.program.items[0] else {
        panic!("binding item");
    };
    let ScalarExpression::Utf8 { value, .. } = &binding.value else {
        panic!("utf8 expression");
    };
    assert_eq!(
        value,
        &vec![b'\\', b'"', b'\'', b'\n', b'\r', b'\t', 0, 0, b'A', 0xF0, 0x9F, 0x98, 0x80]
    );
}

#[test]
fn derives_char_and_artifact_id_with_exact_contextual_types() {
    let result = validate_text(
        r##"%%start
char(char) echo = fn(value) { value };
char initial = '\u{1F600}';
char copied = echo(initial);
artifact_id current = core.this_artifact_id();
bool same = current == current;
%%end"##,
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(initial) = &result.program.items[1] else {
        panic!("char binding");
    };
    assert_eq!(initial.declared_type, ScalarType::Char);
    assert!(matches!(
        initial.value,
        ScalarExpression::Char { value: '😀', .. }
    ));
    let ScalarItem::Binding(current) = &result.program.items[3] else {
        panic!("artifact binding");
    };
    assert_eq!(current.declared_type, ScalarType::ArtifactId);
}

#[test]
fn rejects_invalid_character_shapes_at_literal_spans() {
    for literal in ["''", "'ab'", "'\\q'", "'\\u{D800}'", "'\\u{110000}'"] {
        let text = format!("%%start\nchar value = {literal};\n%%end");
        let literal_start = text.find(literal).expect("literal") as u32;
        let literal_end = literal_start + literal.len() as u32;
        let result = validate_text(&text);
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == "invalid character literal")
            .expect("character diagnostic");
        assert!(diagnostic.labels[0].span.range.start >= literal_start);
        assert!(diagnostic.labels[0].span.range.end <= literal_end);
    }
}

#[test]
fn reports_malformed_string_escapes_at_their_exact_spans() {
    let cases = [
        (r##"\q"##, 1, 3),
        (r##"\u{}"##, 1, 5),
        (r##"\u{12"##, 1, 6),
        (r##"\u{D800}"##, 1, 9),
        (r##"\u{110000}"##, 1, 11),
    ];
    for (literal, relative_start, relative_end) in cases {
        let text = format!("%%start\nstd.utf8 value = \"{}\";\n%%end", literal);
        let parsed = parse_source(source(), text.clone(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let result = derive_scalar_program(&parsed.result);
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == "invalid string literal")
            .expect("string diagnostic");
        let ScalarItem::Binding(binding) = &result.program.items[0] else {
            panic!("binding item");
        };
        let (literal_start, error_span) = match &binding.value {
            ScalarExpression::InvalidInteger {
                span,
                error_span: Some(error_span),
            } => (span.start, *error_span),
            _ => panic!("invalid string expression"),
        };
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(literal_start + relative_start, literal_start + relative_end)
        );
        assert_eq!(error_span, diagnostic.labels[0].span.range);
    }
}

#[test]
fn reports_malformed_extern_module_string_without_panicking() {
    let text = r##"%%start
wasi = extern wasm "\q" { i32(i32, i32) fd_write; };
%%end"##;
    let parsed = parse_source(source(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let result = derive_scalar_program(&parsed.result);
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message == "invalid string literal")
        .expect("extern string diagnostic");
    let ScalarItem::Extern(extern_decl) = &result.program.items[0] else {
        panic!("extern item");
    };
    let start = extern_decl.module_span.start;
    assert_eq!(
        diagnostic.labels[0].span.range,
        ByteSpan::new(start + 1, start + 3)
    );
    assert_eq!(
        extern_decl.actual_module,
        ScalarExternModule::Invalid {
            span: ByteSpan::new(start, start + 4),
            error_span: ByteSpan::new(start + 1, start + 3),
        }
    );
}

#[test]
fn reports_ascii_case_collision_in_project_namespace_scope() {
    let source = module_source("src/main.w");
    let program = module_from_text(
            source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\nMATH = namespace app \"src/other.w\";\n%%end",
        );
    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source, program.items, Vec::new())],
        Vec::new(),
    ));
    let diagnostic = validation
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0008")
        .expect("project case collision diagnostic");
    assert_eq!(diagnostic.labels.len(), 2);
}

#[test]
fn reports_unused_local_and_parameter_at_declaration_spans() {
    let text = "%%start\nunit(i32) f = fn(unused_parameter) { i32 unused_local = 1; };\n%%end";
    let result = validate_text(text);
    let diagnostics: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "B0009")
        .collect();
    assert_eq!(diagnostics.len(), 2);
    let ScalarItem::Function(function) = &result.program.items[0] else {
        panic!("function item");
    };
    let local_span = match &function.body.items[0] {
        ScalarBlockItem::LocalBinding(binding) => binding.span,
        _ => panic!("local binding item"),
    };
    assert_eq!(
        diagnostics[0].labels[0].span.range,
        function.parameter_spans[0]
    );
    assert_eq!(diagnostics[1].labels[0].span.range, local_span);
}

#[test]
fn counts_resolved_uses_across_initializers_assignments_conditions_calls_and_branches() {
    let result = validate_text(
            "%%start\nunit(i32) consume = fn(value) { value; };\ni32(i32) f = fn(input) { i32 local = input; while (local < 2) { local = local + 1; } if (local == 2) { consume(local); } else { consume(input); } local };\n%%end",
        );
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
}

#[test]
fn accepts_documented_declared_identifier_styles_in_single_file() {
    let result = validate_text(
            "%%start\nsnake_namespace = namespace app \"src/library.w\";\nextern_binding = extern wasm \"env\" { i32(i32) extern_function; };\ni32 snake_case = 1;\ni32 PascalCase = 2;\ni32 SCREAMING_SNAKE_CASE = 3;\ni32 _ = 4;\ni32(i32) function_name = fn(_name) { i32 _local = _name; _local };\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
}

#[test]
fn rejects_camel_case_declarations_at_identifier_spans_in_single_file() {
    let text = "%%start\ncamelNamespace = namespace app \"src/library.w\";\ncamelExternBinding = extern wasm \"env\" { i32(i32) camelExternFunction; };\ni32 camelBinding = 1;\ni32(i32) camelFunction = fn(camelParameter) { i32 camelLocal = camelParameter; camelLocal };\n%%end";
    let result = validate_text(text);
    let diagnostics: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.message
                    == "identifier must use snake_case, PascalCase, or SCREAMING_SNAKE_CASE"
        })
        .collect();
    let identifiers = [
        "camelNamespace",
        "camelExternBinding",
        "camelExternFunction",
        "camelBinding",
        "camelFunction",
        "camelParameter",
        "camelLocal",
    ];
    assert_eq!(diagnostics.len(), identifiers.len());
    for identifier in identifiers {
        let start = text.find(identifier).expect("identifier") as u32;
        let span = ByteSpan::new(start, start + identifier.len() as u32);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.labels[0].span.range == span),
            "missing diagnostic for {identifier}"
        );
    }
}

#[test]
fn validates_declared_identifier_styles_in_project() {
    let source = module_source("src/main.w");
    let accepted = module_from_text(
            source.clone(),
            "%%start\nsnake_namespace = namespace app \"src/library.w\";\nextern_binding = extern wasm \"env\" { i32(i32) extern_function; };\ni32 snake_case = 1;\ni32 PascalCase = 2;\ni32 SCREAMING_SNAKE_CASE = 3;\ni32 _ = 4;\ni32(i32) function_name = fn(_name) { i32 _local = _name; _local };\n%%end",
        );
    let accepted = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(
            source.clone(),
            accepted.items,
            Vec::new(),
        )],
        vec![source.clone()],
    ));
    assert!(
        accepted.diagnostics.is_empty(),
        "{:?}",
        accepted.diagnostics
    );

    let text = "%%start\ncamelNamespace = namespace app \"src/library.w\";\ncamelExternBinding = extern wasm \"env\" { i32(i32) camelExternFunction; };\ni32 camelBinding = 1;\ni32(i32) camelFunction = fn(camelParameter) { i32 camelLocal = camelParameter; camelLocal };\n%%end";
    let rejected = module_from_text(source.clone(), text);
    let rejected = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(
            source.clone(),
            rejected.items,
            Vec::new(),
        )],
        vec![source.clone()],
    ));
    let diagnostics: Vec<_> = rejected
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.message
                    == "identifier must use snake_case, PascalCase, or SCREAMING_SNAKE_CASE"
        })
        .collect();
    let identifiers = [
        "camelNamespace",
        "camelExternBinding",
        "camelExternFunction",
        "camelBinding",
        "camelFunction",
        "camelParameter",
        "camelLocal",
    ];
    assert_eq!(diagnostics.len(), identifiers.len());
    for identifier in identifiers {
        let start = text.find(identifier).expect("identifier") as u32;
        let span = ByteSpan::new(start, start + identifier.len() as u32);
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.labels[0].span.source == source
                    && diagnostic.labels[0].span.range == span
            }),
            "missing diagnostic for {identifier}"
        );
    }
}

#[test]
fn validates_static_uses_in_project_path() {
    let source = module_source("src/main.w");
    let program = module_from_text(
        source.clone(),
        "%%start\nunit(i32) f = fn(unused_parameter) { i32 unused_local = 1; };\n%%end",
    );
    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source.clone()],
    ));
    assert_eq!(
        validation
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "B0009")
            .count(),
        2
    );
    assert!(validation
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.labels[0].span.source == source));
}

fn module_source(path: &str) -> SourceIdentity {
    SourceIdentity::new("project".into(), "app".into(), path.into(), "r1".into())
}

fn module_from_text(source: SourceIdentity, text: &str) -> ScalarProgram {
    let parsed = parse_source(source, text.to_owned(), &[]);
    assert!(
        parsed.diagnostics.is_empty(),
        "syntax diagnostics: {:?}",
        parsed.diagnostics
    );
    derive_scalar_program(&parsed.result).program
}

#[test]
fn resolves_qualified_member_only_through_bound_module() {
    let math_source = module_source("src/math.w");
    let math = module_from_text(
        math_source.clone(),
        "%%start\ni32(i32, i32) add = fn(left, right) { left + right };\n%%end",
    );
    let main_source = module_source("src/main.w");
    let main = module_from_text(
        main_source.clone(),
        "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.add(20, 22);\n%%end",
    );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let project = ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "math".to_owned(),
                    target: math_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(math_source.clone(), math.items, Vec::new()),
        ],
        vec![main_source, math_source],
    );
    let result = validate_scalar_project(project);
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
}

#[test]
fn validates_two_output_namespace_call_and_receiver_order() {
    let math_source = module_source("src/math.w");
    let math = module_from_text(
        math_source.clone(),
        "%%start\n(u64, bool)() pair = fn { 1 };\n%%end",
    );
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\nu64 first, bool second = math.pair();\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let project = ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "math".to_owned(),
                    target: math_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(math_source.clone(), math.items, Vec::new()),
        ],
        vec![main_source.clone(), math_source],
    );
    let result = validate_scalar_project(project);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(binding) = &result.project.modules[0].items[1] else {
        panic!("namespace binding")
    };
    assert_eq!(
        binding.output_origin,
        ScalarBindingOutputOrigin::SingleExpression
    );
    assert_eq!(binding.output_sequence.outputs.len(), 2);
    let call_span = expression_span(&binding.value);
    assert_eq!(binding.output_sequence.outputs[0].ty, ScalarType::U64);
    assert_eq!(binding.output_sequence.outputs[0].span, call_span);
    assert_eq!(binding.output_sequence.outputs[1].ty, ScalarType::Bool);
    assert_eq!(binding.output_sequence.outputs[1].span, call_span);
    assert_eq!(binding.output_values[0].position, 0);
    assert_eq!(binding.output_values[1].position, 1);
    assert_eq!(binding.output_values[0].ty, ScalarType::U64);
    assert_eq!(binding.output_values[1].ty, ScalarType::Bool);
    assert_eq!(expression_span(&binding.output_values[0].value), call_span);
    assert_eq!(expression_span(&binding.output_values[1].value), call_span);
    assert_eq!(binding.receivers[0].ty, ScalarType::U64);
    assert_eq!(binding.receivers[1].ty, ScalarType::Bool);
}

#[test]
fn resolves_direct_root_extern_with_local_namespace_present() {
    let local_source = module_source("src/local.w");
    let local = module_from_text(local_source.clone(), "%%start\ni32 value = 7;\n%%end");
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nwasi = extern wasm \"wasi_snapshot_preview1\" { i32(i32, i32) fd_write; };\nlocal = namespace app \"src/local.w\";\ni32 value = local.value;\ni32 out = wasi.fd_write(1, 1);\n%%end",
        );
    let namespace_span = match &main.items[1] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let project = ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "local".to_owned(),
                    target: local_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(local_source.clone(), local.items, Vec::new()),
        ],
        vec![main_source, local_source],
    );
    let result = validate_scalar_project(project);
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
}

#[test]
fn validates_project_namespace_uniqueness_and_preserves_distinct_targets() {
    let first_source = module_source("src/first.w");
    let second_source = module_source("src/second.w");
    let main_source = module_source("src/main.w");
    let duplicate = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/first.w\";\nmath = namespace app \"src/second.w\";\n%%end",
        );
    let duplicate_bindings = vec![
        match &duplicate.items[0] {
            ScalarItem::Namespace(namespace) => ScalarNamespaceBinding {
                binding: namespace.binding.clone(),
                target: first_source.clone(),
                span: namespace.span,
            },
            _ => panic!("namespace item"),
        },
        match &duplicate.items[1] {
            ScalarItem::Namespace(namespace) => ScalarNamespaceBinding {
                binding: namespace.binding.clone(),
                target: second_source.clone(),
                span: namespace.span,
            },
            _ => panic!("namespace item"),
        },
    ];
    let second_span = duplicate_bindings[1].span;
    let duplicate_result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(main_source.clone(), duplicate.items, duplicate_bindings),
            ScalarModule::new(
                first_source.clone(),
                module_from_text(first_source.clone(), "%%start\ni32 value = 1;\n%%end").items,
                Vec::new(),
            ),
            ScalarModule::new(
                second_source.clone(),
                module_from_text(second_source.clone(), "%%start\ni32 value = 2;\n%%end").items,
                Vec::new(),
            ),
        ],
        vec![
            main_source.clone(),
            first_source.clone(),
            second_source.clone(),
        ],
    ));
    let diagnostics: Vec<_> = duplicate_result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "B0002")
        .collect();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].labels[0].span.range, second_span);

    let distinct = module_from_text(
            main_source.clone(),
            "%%start\nfirst = namespace app \"src/first.w\";\nsecond = namespace app \"src/second.w\";\n%%end",
        );
    let distinct_bindings = vec![
        ScalarNamespaceBinding {
            binding: "first".to_owned(),
            target: first_source.clone(),
            span: match &distinct.items[0] {
                ScalarItem::Namespace(namespace) => namespace.span,
                _ => panic!("namespace item"),
            },
        },
        ScalarNamespaceBinding {
            binding: "second".to_owned(),
            target: second_source.clone(),
            span: match &distinct.items[1] {
                ScalarItem::Namespace(namespace) => namespace.span,
                _ => panic!("namespace item"),
            },
        },
    ];
    let distinct_result = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(
            main_source,
            distinct.items,
            distinct_bindings.clone(),
        )],
        Vec::new(),
    ));
    assert!(distinct_result.diagnostics.is_empty());
    assert_eq!(
        distinct_result.project.modules[0].namespace_bindings,
        distinct_bindings
    );
}

#[test]
fn reports_unknown_member_at_member_span_and_target_source() {
    let math_source = module_source("src/math.w");
    let math = module_from_text(math_source.clone(), "%%start\ni32 value = 1;\n%%end");
    let main_source = module_source("src/main.w");
    let main = module_from_text(
        main_source.clone(),
        "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.add(20, 22);\n%%end",
    );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let project = ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "math".to_owned(),
                    target: math_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(math_source.clone(), math.items, Vec::new()),
        ],
        vec![main_source.clone(), math_source.clone()],
    );
    let result = validate_scalar_project(project);
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "M0002")
        .expect("unknown member diagnostic");
    assert!(!result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0003"));
    assert_eq!(diagnostic.labels[0].span.source, main_source);
    assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(61, 64));
    assert_eq!(diagnostic.labels[1].span.source, math_source);
}

#[test]
fn resolves_qualified_member_read_and_assignment_through_bound_module() {
    let math_source = module_source("src/math.w");
    let math = module_from_text(math_source.clone(), "%%start\ni32 value = 1;\n%%end");
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.value;\nmath.value = result;\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let read = match &main.items[1] {
        ScalarItem::Binding(binding) => &binding.value,
        _ => panic!("result binding"),
    };
    let ScalarExpression::Member {
        receiver,
        name,
        enum_tag: _,
        receiver_span,
        name_span,
        span,
    } = read
    else {
        panic!("qualified member read");
    };
    assert_eq!(receiver, "math");
    assert_eq!(name, "value");
    assert_eq!(*receiver_span, ByteSpan::new(56, 60));
    assert_eq!(*name_span, ByteSpan::new(61, 66));
    assert_eq!(*span, ByteSpan::new(56, 66));
    let ScalarItem::Executable(ScalarBlockItem::Assignment(assignment)) = &main.items[2] else {
        panic!("qualified member assignment");
    };
    assert_eq!(assignment.receiver.as_deref(), Some("math"));
    assert_eq!(assignment.target, "value");
    assert_eq!(assignment.receiver_span, Some(ByteSpan::new(68, 72)));
    assert_eq!(assignment.target_span, ByteSpan::new(73, 78));
    let result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "math".to_owned(),
                    target: math_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(math_source, math.items, Vec::new()),
        ],
        Vec::new(),
    ));
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
}

#[test]
fn reports_unknown_qualified_member_read_and_assignment_at_member_span() {
    let math_source = module_source("src/math.w");
    let math = module_from_text(math_source.clone(), "%%start\ni32 value = 1;\n%%end");
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.missing;\nmath.missing = result;\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "math".to_owned(),
                    target: math_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(math_source.clone(), math.items, Vec::new()),
        ],
        Vec::new(),
    ));
    let diagnostics: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "M0002")
        .collect();
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].labels[0].span.range, ByteSpan::new(61, 68));
    assert_eq!(diagnostics[1].labels[0].span.range, ByteSpan::new(75, 82));
    assert_eq!(diagnostics[0].labels[1].span.source, math_source);
    assert!(!result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0003"));
}

#[test]
fn preserves_sibling_diagnostics_after_error_namespace_member() {
    let math_source = module_source("src/math.w");
    let math = module_from_text(math_source.clone(), "%%start\ni32 value = 1;\n%%end");
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nmath = namespace app \"src/math.w\";\ni32 missing = math.missing;\ni32 invalid = true;\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source,
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "math".to_owned(),
                    target: math_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(math_source, math.items, Vec::new()),
        ],
        Vec::new(),
    ));
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "M0002"));
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0003"));
}

#[test]
fn rejects_importer_unqualified_member_lookup() {
    let math_source = module_source("src/math.w");
    let math = module_from_text(
        math_source.clone(),
        "%%start\ni32(i32, i32) add = fn(left, right) { left + right };\n%%end",
    );
    let main_source = module_source("src/main.w");
    let main = module_from_text(
        main_source.clone(),
        "%%start\nmath = namespace app \"src/math.w\";\ni32 result = add(20, 22);\n%%end",
    );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source,
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "math".to_owned(),
                    target: math_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(math_source, math.items, Vec::new()),
        ],
        Vec::new(),
    ));
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0001"));
}

#[test]
fn represents_source_ordered_module_initialization_once() {
    let first_source = module_source("src/first.w");
    let second_source = module_source("src/second.w");
    let first = ScalarModule::new(
        first_source.clone(),
        module_from_text(first_source.clone(), "%%start\ni32 first = 1;\n%%end").items,
        Vec::new(),
    );
    let second = ScalarModule::new(
        second_source.clone(),
        module_from_text(second_source.clone(), "%%start\ni32 second = 2;\n%%end").items,
        Vec::new(),
    );
    assert_eq!(first.initialization_nodes.len(), 1);
    assert_eq!(second.initialization_nodes.len(), 1);
    let project = ScalarProject::new(
        vec![first, second],
        vec![
            second_source.clone(),
            first_source.clone(),
            second_source.clone(),
        ],
    );
    assert_eq!(
        project.initialization_order,
        vec![second_source, first_source]
    );
}

#[test]
fn derives_struct_layout_fields_and_raw_address() {
    let result = validate_text(
            "%%start\nstruct WasiIovec {\n\t*?u8 buf;\n\tu32 len;\n}\nunsafe {\n\tWasiIovec item = {\n\t\t.buf = null;\n\t\t.len = 0;\n\t};\n\t*?WasiIovec address = &?item;\n};\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.program.structs[0].fields[0].offset, Some(0));
    assert_eq!(result.program.structs[0].fields[1].offset, Some(4));
    assert_eq!(result.program.structs[0].layout.as_ref().unwrap().size, 8);
    assert_eq!(result.program.target_layout, ScalarTargetLayout::WASM32);
    assert_eq!(
        result.program.structs[0].id,
        ScalarStructId {
            source: source(),
            index: 0
        }
    );
    assert_eq!(
        result.program.structs[0].fields[0].id,
        ScalarStructFieldId {
            structure: ScalarStructId {
                source: source(),
                index: 0
            },
            index: 0,
        }
    );
    let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
        &result.program.items[0]
    else {
        panic!("unsafe block")
    };
    let ScalarBlockItem::LocalBinding(address) = &block.items[1] else {
        panic!("address binding")
    };
    assert!(matches!(address.value, ScalarExpression::RawAddress { .. }));
}

#[test]
fn rejects_string_literal_without_std_context() {
    let result = validate_text("%%start\ni32 value = \"text\";\n%%end");
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("std.utf8 context")));
}

#[test]
fn discards_unreceived_outputs_in_scalar_programs() {
    for text in [
            "%%start\n(i32, u64)() pair = fn { 1 };\npair();\n%%end",
            "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first = pair();\n%%end",
            "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first, u64 second = pair();\n%%end",
            "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first = 0;\nfirst = pair();\n%%end",
            "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first = 0;\nu64 second = 0;\nfirst, second = pair();\n%%end",
        ] {
            let valid = validate_text(text);
            assert!(
                valid.diagnostics.is_empty(),
                "{text}: {:?}",
                valid.diagnostics
            );
        }

    let wrong_binding =
        validate_text("%%start\n(i32, u64)() pair = fn { 1 };\nbool first = pair();\n%%end");
    assert!(wrong_binding.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "expression type does not match expected type"
    }));
    let wrong_assignment = validate_text(
        "%%start\n(i32, u64)() pair = fn { 1 };\nbool first = false;\nfirst = pair();\n%%end",
    );
    assert!(wrong_assignment.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "expression type does not match expected type"
    }));

    for (text, message) in [
            (
                "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first, u64 second, bool third = pair();\n%%end",
                "call has fewer outputs than receivers",
            ),
            (
                "%%start\n(i32, u64)() pair = fn { 1 };\ni32 first = 0;\nu64 second = 0;\nbool third = false;\nfirst, second, third = pair();\n%%end",
                "call has fewer outputs than assignment targets",
            ),
        ] {
            let invalid = validate_text(text);
            assert!(
                invalid
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message == message)
            );
            assert!(!invalid.diagnostics.iter().any(|diagnostic| {
                diagnostic.message == "call has more outputs than receivers"
                    || diagnostic.message == "call has more outputs than assignment targets"
            }));
        }
}

#[test]
fn discards_unreceived_outputs_in_namespace_resolved_projects() {
    let std_source = module_source("src/std.w");
    let std = module_from_text(
        std_source.clone(),
        "%%start\n(i32, u64)() print = fn { 1 };\n%%end",
    );
    let validate_project_text = |text: &str| {
        let main_source = module_source("src/main.w");
        let main = module_from_text(main_source.clone(), text);
        let namespace_span = match &main.items[0] {
            ScalarItem::Namespace(namespace) => namespace.span,
            _ => panic!("namespace item"),
        };
        validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::new(
                    main_source,
                    main.items,
                    vec![ScalarNamespaceBinding {
                        binding: "std".to_owned(),
                        target: std_source.clone(),
                        span: namespace_span,
                    }],
                ),
                ScalarModule::new(std_source.clone(), std.items.clone(), Vec::new()),
            ],
            Vec::new(),
        ))
    };

    for text in [
            "%%start\nstd = namespace app \"src/std.w\";\nstd.print();\n%%end",
            "%%start\nstd = namespace app \"src/std.w\";\ni32 first = std.print();\n%%end",
            "%%start\nstd = namespace app \"src/std.w\";\ni32 first, u64 second = std.print();\n%%end",
            "%%start\nstd = namespace app \"src/std.w\";\ni32 first = 0;\nfirst = std.print();\n%%end",
            "%%start\nstd = namespace app \"src/std.w\";\ni32 first = 0;\nu64 second = 0;\nfirst, second = std.print();\n%%end",
        ] {
            let valid = validate_project_text(text);
            assert!(
                valid.diagnostics.is_empty(),
                "{text}: {:?}",
                valid.diagnostics
            );
        }

    let wrong_type = validate_project_text(
        "%%start\nstd = namespace app \"src/std.w\";\nbool first = std.print();\n%%end",
    );
    assert!(wrong_type.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "expression type does not match expected type"
    }));

    for (text, message) in [
            (
                "%%start\nstd = namespace app \"src/std.w\";\ni32 first, u64 second, bool third = std.print();\n%%end",
                "call has fewer outputs than receivers",
            ),
            (
                "%%start\nstd = namespace app \"src/std.w\";\ni32 first = 0;\nu64 second = 0;\nbool third = false;\nfirst, second, third = std.print();\n%%end",
                "call has fewer outputs than assignment targets",
            ),
        ] {
            let invalid = validate_project_text(text);
            assert!(
                invalid
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message == message)
            );
            assert!(!invalid.diagnostics.iter().any(|diagnostic| {
                diagnostic.message == "call has more outputs than receivers"
                    || diagnostic.message == "call has more outputs than assignment targets"
            }));
        }
}

#[test]
fn preserves_independent_output_expression_spans() {
    let result = validate_text("%%start\nu64 first, bool second = 1, true;\n%%end");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(binding) = &result.program.items[0] else {
        panic!("output binding");
    };
    assert_eq!(
        binding.output_origin,
        ScalarBindingOutputOrigin::IndependentExpressions
    );
    assert_eq!(binding.output_values[0].position, 0);
    assert_eq!(binding.output_values[1].position, 1);
    assert_ne!(
        expression_span(&binding.output_values[0].value),
        expression_span(&binding.output_values[1].value)
    );
    assert_eq!(
        binding.output_sequence.outputs[0].span,
        binding.output_values[0].span
    );
    assert_eq!(
        binding.output_sequence.outputs[1].span,
        binding.output_values[1].span
    );
}

#[test]
fn marks_equal_independent_output_expressions_independently() {
    let result = validate_text("%%start\nu64 first, u64 second = 1, 1;\n%%end");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(binding) = &result.program.items[0] else {
        panic!("output binding");
    };
    assert_eq!(
        binding.output_origin,
        ScalarBindingOutputOrigin::IndependentExpressions
    );
    assert!(matches!(
        (
            &binding.output_values[0].value,
            &binding.output_values[1].value
        ),
        (
            ScalarExpression::Integer { value: first, .. },
            ScalarExpression::Integer { value: second, .. }
        ) if first == second
    ));
    assert_ne!(
        expression_span(&binding.output_values[0].value),
        expression_span(&binding.output_values[1].value)
    );
}

#[test]
fn preserves_one_output_call_contexts_for_strict_integers_and_null() {
    let result = validate_text(
            "%%start\nu32(u32) identity = fn(value) { value };\nu32 number = identity(1);\nu32(*?u8) keep = fn(value) { if (value == null) { 1 } else { 1 } };\nu32 result = keep(null);\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(binding) = &result.program.items[1] else {
        panic!("one-output binding");
    };
    assert_eq!(
        binding.output_origin,
        ScalarBindingOutputOrigin::SingleExpression
    );
}

#[test]
fn rejects_typed_i32_for_u32_and_contextually_types_u32_literals() {
    let declaration = validate_text("%%start\ni32 value = 1;\nu32 result = value;\n%%end");
    assert_eq!(
        declaration
            .diagnostics
            .iter()
            .filter(
                |diagnostic| diagnostic.message == "expression type does not match expected type"
            )
            .count(),
        1
    );

    let assignment =
        validate_text("%%start\ni32 value = 1;\nu32 target = 2;\ntarget = value;\n%%end");
    assert!(assignment
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "expression type does not match expected type"));

    let callable = validate_text(
            "%%start\nu32(u32) identity = fn(value) { value };\ni32 value = 1;\nu32 result = identity(value);\n%%end",
        );
    assert!(callable
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "expression type does not match expected type"));

    let literal = validate_text("%%start\nu32 value = 1 + 2;\n%%end");
    assert!(literal.diagnostics.is_empty(), "{:?}", literal.diagnostics);
    let out_of_range = validate_text("%%start\nu32 value = 4294967296;\n%%end");
    assert!(out_of_range
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0010"));
}

#[test]
fn resolves_contextual_null_in_declarations_assignments_calls_and_nested_expressions() {
    let text = "%%start\nu32(*?u8) keep = fn(value) { if (value == null) { 1 } else { 1 } };\n*?u8 pointer = null;\npointer = null;\nu32 result = keep(null);\n*?u8 nested = if (true) { if (true) { null } else { null } } else { null };\nbool same = pointer == null;\n%%end";
    let result = validate_text(text);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

    let ScalarItem::Binding(binding) = &result.program.items[1] else {
        panic!("pointer declaration");
    };
    assert_eq!(
        binding.declared_type,
        ScalarType::RawPointer(Box::new(ScalarType::U8))
    );
    let ScalarItem::Binding(nested) = &result.program.items[4] else {
        panic!("nested pointer declaration");
    };
    assert_eq!(
        nested.declared_type,
        ScalarType::RawPointer(Box::new(ScalarType::U8))
    );
}

#[test]
fn reports_unconstrained_null_once_at_the_null_token_span() {
    let cases = [
        "%%start\nnull;\n%%end",
        "%%start\ni32 value = 1;\nvalue = null;\n%%end",
        "%%start\ni32(i32) identity = fn(value) { value };\ni32 result = identity(null);\n%%end",
        "%%start\ni32 value = if (true) { null } else { 1 };\n%%end",
    ];
    for text in cases {
        let result = validate_text(text);
        let diagnostics: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message == "null requires a pointer context")
            .collect();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let start = text.find("null").expect("null token") as u32;
        assert_eq!(diagnostics[0].labels.len(), 1);
        assert_eq!(
            diagnostics[0].labels[0].span.range,
            ByteSpan::new(start, start + 4)
        );
    }
}

#[test]
fn project_validation_applies_exact_types_and_contextual_null() {
    let dependency_source = module_source("src/dependency.w");
    let dependency = module_from_text(
        dependency_source.clone(),
        "%%start\nu32(*?u8) accept = fn(value) { 1 };\ni32 value = 1;\n%%end",
    );
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\ndependency = namespace app \"src/dependency.w\";\ni32 value = 1;\nu32 rejected = dependency.accept(value);\nu32 nested = 1 + (2 + 3);\n*?u8 pointer = null;\n*?u8 selected = if (true) { null } else { null };\nbool same = pointer == (if (true) { null } else { null });\nbool nested_comparison = (pointer == null) == true;\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source.clone(),
                main.items,
                vec![ScalarNamespaceBinding {
                    binding: "dependency".to_owned(),
                    target: dependency_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::new(dependency_source, dependency.items, Vec::new()),
        ],
        vec![main_source],
    ));
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "expression type does not match expected type"));
    assert!(!result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "null requires a pointer context"));
}

#[test]
fn resolves_forward_struct_types_and_canonical_field_places() {
    let result = validate_text(
            "%%start\nstruct Outer {\n\tInner inner;\n\t*?Inner pointer;\n}\nstruct Inner {\n\tu32 value;\n}\nunsafe {\n\tOuter item = { .inner = { .value = 1; }; .pointer = null; };\n\t*?Outer address = &?item;\n\t*?u32 value_address = &?(*item.pointer).value;\n\t*?Inner pointer_value_address = &?(*address).inner;\n};\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(
        result.program.structs[0].id,
        ScalarStructId {
            source: source(),
            index: 0
        }
    );
    assert_eq!(
        result.program.structs[1].id,
        ScalarStructId {
            source: source(),
            index: 1
        }
    );
    assert_eq!(
        result.program.structs[0].fields[0].ty,
        ScalarType::Struct(ScalarStructId {
            source: source(),
            index: 1
        })
    );
    let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
        &result.program.items[0]
    else {
        panic!("unsafe block")
    };
    let ScalarBlockItem::LocalBinding(value_address) = &block.items[2] else {
        panic!("value address binding")
    };
    let ScalarExpression::RawAddress { place, .. } = &value_address.value else {
        panic!("raw value address")
    };
    let ScalarPlace::Field { field, base, .. } = place else {
        panic!("nested field place")
    };
    assert_eq!(
        *field,
        ScalarFieldReference::Resolved(ScalarStructFieldId {
            structure: ScalarStructId {
                source: source(),
                index: 1
            },
            index: 0,
        })
    );
    let ScalarPlace::Dereference { .. } = base.as_ref() else {
        panic!("pointer dereference place")
    };
}

#[test]
fn validates_struct_literals_in_nested_positions_and_pointer_null_fields() {
    let result = validate_text(
            "%%start\nstruct Pair {\n\tu32 left;\n\t*?u8 right;\n}\nPair value = if (true) { { .left = 1; .right = null; } } else { { .left = 0; .right = null; } };\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(binding) = &result.program.items[0] else {
        panic!("struct binding")
    };
    assert!(matches!(binding.value, ScalarExpression::If { .. }));
    assert_eq!(
        binding.declared_type,
        ScalarType::Struct(ScalarStructId {
            source: source(),
            index: 0
        })
    );
}

#[test]
fn reports_struct_literal_field_shape_errors_at_field_spans() {
    let cases = [
        (
            ".left = 1; .left = 2; .right = null;",
            "duplicate struct literal field",
            ".left = 2",
            4,
        ),
        (
            ".left = 1; .unknown = 2;",
            "unknown struct field",
            ".unknown",
            7,
        ),
        (
            ".left = 1;",
            "struct literal must initialize every field",
            "{ .left",
            0,
        ),
    ];
    for (fields, message, span_text, field_length) in cases {
        let text = format!(
                "%%start\nstruct Pair {{\n\tu32 left;\n\tu32 right;\n}}\nPair value = {{ {fields} }};\n%%end"
            );
        let result = validate_text(&text);
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == message)
            .expect("struct literal diagnostic");
        assert_eq!(diagnostic.labels[0].message, message);
        if message == "struct literal must initialize every field" {
            let ScalarItem::Binding(binding) = &result.program.items[0] else {
                panic!("struct binding")
            };
            let ScalarExpression::StructLiteral { span, .. } = &binding.value else {
                panic!("struct literal")
            };
            assert_eq!(diagnostic.labels[0].span.range, *span);
        } else {
            let start = text.find(span_text).expect("diagnostic span text") as u32 + 1;
            let end = start + field_length;
            assert_eq!(diagnostic.labels[0].span.range, ByteSpan::new(start, end));
        }
    }
}

#[test]
fn enforces_unsafe_raw_addresses_and_rejects_invalid_field_receivers() {
    let safe = validate_text(
            "%%start\nstruct Pair {\n\tu32 value;\n}\nPair item = { .value = 1; };\n*?Pair address = &?item;\n%%end",
        );
    let diagnostic = safe
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0012")
        .expect("raw address safety diagnostic");
    assert_eq!(diagnostic.message, "raw address requires an unsafe block");
    let invalid = validate_text(
        "%%start\ni32 value = 1;\nunsafe {\n\t*?i32 address = &?value.missing;\n};\n%%end",
    );
    let diagnostic = invalid
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message == "value has no field")
        .expect("invalid field receiver diagnostic");
    let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
        &invalid.program.items[1]
    else {
        panic!("unsafe block")
    };
    let ScalarBlockItem::LocalBinding(binding) = &block.items[0] else {
        panic!("address binding")
    };
    let ScalarExpression::RawAddress { place, .. } = &binding.value else {
        panic!("raw address")
    };
    assert_eq!(
        diagnostic.labels[0].span.range,
        match place {
            ScalarPlace::Field { span, .. } => *span,
            _ => panic!("field place"),
        }
    );
}

#[test]
fn derives_checked_reference_addresses_with_exact_shared_mutable_and_aggregate_types() {
    let result = validate_text(
            "%%start\nstruct Item {\n\tu8 field;\n}\nu8 value = 1;\nItem record = { .field = 2; };\nu8[4] bytes = [3, 4, 5, 6];\n*u8 shared = &value;\n*!u8 mutable = &!value;\n*u8 field = &record.field;\n*(u8[4]) array = &bytes;\n*!(u8[4]) mutable_array = &!bytes;\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

    for (item, mutability, array) in [
        (3, ScalarReferenceMutability::Shared, false),
        (4, ScalarReferenceMutability::Mutable, false),
        (5, ScalarReferenceMutability::Shared, false),
        (6, ScalarReferenceMutability::Shared, true),
        (7, ScalarReferenceMutability::Mutable, true),
    ] {
        let item = &result.program.items[item];
        let ScalarItem::Binding(binding) = item else {
            panic!("checked reference binding")
        };
        assert!(matches!(
            &binding.declared_type,
            ScalarType::CheckedReference { mutability: actual, inner }
                if *actual == mutability
                    && if array {
                        matches!(
                            inner.as_ref(),
                            ScalarType::Array { element, length: 4, .. }
                                if element.as_ref() == &ScalarType::U8
                        )
                    } else {
                        inner.as_ref() == &ScalarType::U8
                    }
        ));
        assert!(matches!(
            binding.value,
            ScalarExpression::CheckedAddress { .. }
        ));
    }
}

#[test]
fn derives_checked_reference_dereference_reads_and_rejects_deferred_targets() {
    let result = validate_text(
            "%%start\nstruct Record {\n\tu8 first;\n\tu32 second;\n}\nu32 value = 7;\nRecord item = { .first = 1; .second = 2; };\n*u32 shared = &value;\n*!u32 mutable = &!value;\n*Record record_shared = &item;\nu32 shared_value = *shared;\nu32 mutable_value = *mutable;\nu32 field_value = (*record_shared).second;\n%%end",
        );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    for index in [5, 6, 7] {
        let ScalarItem::Binding(binding) = &result.program.items[index] else {
            panic!("checked dereference binding")
        };
        assert!(matches!(
            binding.value,
            ScalarExpression::Dereference { .. }
        ));
    }

    let invalid_text = "%%start\nu32 value = 7;\n*?u32 raw = null;\n*unit unit = null;\nu32 non_reference = *value;\nu32 raw_read = *raw;\nunit unit_read = *unit;\n%%end";
    let invalid = validate_text(invalid_text);
    for message in [
        "cannot dereference value",
        "checked dereference requires a checked reference",
        "cannot read unit through checked reference",
    ] {
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == message),
            "{:?}",
            invalid.diagnostics
        );
    }
    for (message, spelling) in [
        ("cannot dereference value", "*value"),
        ("checked dereference requires a checked reference", "*raw"),
        ("cannot read unit through checked reference", "*unit"),
    ] {
        let start = invalid_text.rfind(spelling).expect("dereference") as u32;
        let diagnostic = invalid
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == message)
            .expect("dereference diagnostic");
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + spelling.len() as u32)
        );
    }

    let invalid_field_text = "%%start\nstruct Record {\n\tunit marker;\n\tu32 value;\n}\nRecord item = { .marker = 1; .value = 2; };\n*Record reference = &item;\nunit marker = (*reference).marker;\nu32 missing = (*reference).missing;\n%%end";
    let invalid_field = validate_text(invalid_field_text);
    for (message, field) in [
        ("cannot read unit through checked reference", "marker"),
        ("value has no field", "missing"),
    ] {
        let start = invalid_field_text.rfind(field).expect("field") as u32;
        let diagnostic = invalid_field
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == message)
            .expect("field diagnostic");
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + field.len() as u32)
        );
    }

    let aggregate = validate_text(
            "%%start\nstruct copy Record {\n\tu32 value;\n}\nRecord sample = { .value = 7; };\n*Record reference = &sample;\nRecord read = *reference;\n%%end",
        );
    assert!(
        aggregate.diagnostics.is_empty(),
        "{:?}",
        aggregate.diagnostics
    );

    let null_reference =
        validate_text("%%start\n*u32 reference = null;\nu32 value = *reference;\n%%end");
    assert!(
        null_reference.diagnostics.is_empty(),
        "{:?}",
        null_reference.diagnostics
    );
}

#[test]
fn reports_project_dereference_field_errors_at_the_field_token() {
    let child_source = module_source("src/child.w");
    let child = module_from_text(
        child_source.clone(),
        "%%start\nstruct Record {\n\tunit marker;\n\tu32 value;\n}\n%%end",
    );
    let root_source = module_source("src/main.w");
    let root_text = "%%start\nchild = namespace app \"src/child.w\";\nu32 value = 1;\n*?u32 raw = null;\n*unit unit = null;\nu32 invalid_value = *value;\nu32 invalid_raw = *raw;\nunit invalid_unit = *unit;\nchild.Record item = { .marker = 1; .value = 1; };\n*child.Record reference = &item;\nunit marker = (*reference).marker;\nu32 result = (*reference).missing;\n%%end";
    let root = module_from_text(root_source.clone(), root_text);
    let namespace = match &root.items[0] {
        ScalarItem::Namespace(namespace) => (namespace.binding.clone(), namespace.span),
        _ => panic!("namespace"),
    };
    let result = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                root,
                vec![ScalarNamespaceBinding {
                    binding: namespace.0,
                    target: child_source.clone(),
                    span: namespace.1,
                }],
            ),
            ScalarModule::from_program(child, Vec::new()),
        ],
        vec![root_source, child_source],
    ));
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message == "value has no field")
        .expect("field diagnostic");
    let start = root_text.rfind("missing").expect("field") as u32;
    assert_eq!(
        diagnostic.labels[0].span.range,
        ByteSpan::new(start, start + "missing".len() as u32)
    );
    for (message, spelling) in [
        ("cannot dereference value", "*value"),
        ("checked dereference requires a checked reference", "*raw"),
        ("cannot read unit through checked reference", "*unit"),
    ] {
        let start = root_text.rfind(spelling).expect("dereference") as u32;
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == message)
            .expect("dereference diagnostic");
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + spelling.len() as u32)
        );
    }
    let marker_start = root_text.rfind("marker").expect("field") as u32;
    let marker = result
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.message == "cannot read unit through checked reference"
                && diagnostic.labels[0].span.range
                    == ByteSpan::new(marker_start, marker_start + "marker".len() as u32)
        })
        .expect("unit field diagnostic");
    assert_eq!(
        marker.labels[0].span.range,
        ByteSpan::new(marker_start, marker_start + "marker".len() as u32)
    );
}

#[test]
fn rejects_checked_reference_mutability_mismatches_and_unsupported_addresses() {
    let mismatch = validate_text(
        "%%start\nu8 value = 1;\n*!u8 shared = &value;\n*u8 mutable = &!value;\n%%end",
    );
    assert_eq!(
        mismatch
            .diagnostics
            .iter()
            .filter(
                |diagnostic| diagnostic.message == "expression type does not match expected type"
            )
            .count(),
        2,
        "{:?}",
        mismatch.diagnostics
    );

    let unsupported = validate_text(
            "%%start\nstruct Record {\n\tu8 field;\n}\n*?Record pointer = null;\n*u8 field = &(*pointer).field;\n%%end",
        );
    assert!(unsupported.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "checked address requires a storage name or direct storage field"
    }));

    let callable =
        validate_text("%%start\nu8() make = fn { 1 };\n*(u8()) reference = &make;\n%%end");
    assert!(callable.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "checked address requires a storage name or direct storage field"
    }));

    let unit = validate_text(
            "%%start\nunit() touch = fn { 1; };\nunit value = touch();\n*unit reference = &value;\n%%end",
        );
    assert!(unit.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "checked address requires a storage name or direct storage field"
    }));
    assert_eq!(
        crate::emit_scalar_llvm(&unit).unwrap_err(),
        "cannot emit LLVM for an invalid scalar program"
    );

    let unit_field = validate_text(
            "%%start\nstruct Record {\n\tunit field;\n}\nunit() touch = fn { 1; };\nRecord record = { .field = touch(); };\n*unit reference = &record.field;\n%%end",
        );
    assert!(unit_field.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "checked address requires a storage name or direct storage field"
    }));
}

#[test]
fn retains_raw_address_safety_and_rejects_checked_references_in_extern_signatures() {
    let raw = validate_text("%%start\nu8 value = 1;\n*?u8 address = &?value;\n%%end");
    assert!(raw.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "B0012" && diagnostic.message == "raw address requires an unsafe block"
    }));

    let externs = validate_text(
        "%%start\nenv = extern wasm \"env\" { unit(*u8) consume; *!u8() produce; };\n%%end",
    );
    assert_eq!(
        externs
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message
                == "direct FFI checked references are not supported")
            .count(),
        2,
        "{:?}",
        externs.diagnostics
    );
}

#[test]
fn validates_structs_through_project_modules() {
    let source = module_source("src/main.w");
    let program = module_from_text(
            source.clone(),
            "%%start\nstruct Outer {\n\tPair pair;\n}\nstruct Pair {\n\tu32 value;\n}\nraw = extern wasm \"env\" { unit(*?Pair) touch; };\nunit(Outer) consume = fn(value) { unsafe { *?Outer pointer = &?value; *?Pair field_address = &?(*pointer).pair; raw.touch(field_address); }; };\nOuter item = { .pair = { .value = 1; }; };\n%%end",
        );
    let struct_id = program.structs[0].id.clone();
    let pair_id = program.structs[1].id.clone();
    let struct_layout = program.structs[0].layout.clone();
    let result = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program, Vec::new())],
        vec![source.clone()],
    ));
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.project.modules[0].structs[0].id, struct_id.clone());
    assert_eq!(result.project.modules[0].structs[0].layout, struct_layout);
    let ScalarItem::Extern(extern_decl) = &result.project.modules[0].items[0] else {
        panic!("extern declaration");
    };
    let ScalarType::Callable { parameters, .. } = &extern_decl.functions[0].signature else {
        panic!("extern signature");
    };
    assert!(
        matches!(parameters[0], ScalarType::RawPointer(ref inner) if inner.as_ref() == &ScalarType::Struct(pair_id.clone()))
    );
    let ScalarItem::Function(function) = &result.project.modules[0].items[1] else {
        panic!("callable declaration");
    };
    let ScalarType::Callable { parameters, .. } = &function.signature else {
        panic!("callable signature");
    };
    assert_eq!(parameters, &[ScalarType::Struct(struct_id.clone())]);
    let ScalarBlockItem::Expression(ScalarExpression::Block(block)) = &function.body.items[0]
    else {
        panic!("unsafe block");
    };
    let ScalarBlockItem::LocalBinding(binding) = &block.items[1] else {
        panic!("field address binding");
    };
    let ScalarExpression::RawAddress {
        place: ScalarPlace::Field { field, .. },
        ..
    } = &binding.value
    else {
        panic!("field address");
    };
    assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == struct_id));
    let ScalarItem::Binding(binding) = &result.project.modules[0].items[2] else {
        panic!("struct binding");
    };
    assert_eq!(binding.declared_type, ScalarType::Struct(struct_id));
}

#[test]
fn resolves_imported_struct_identity_for_literals_and_field_addresses() {
    let child_source = module_source("src/child.w");
    let child_program = module_from_text(
        child_source.clone(),
        "%%start\nstruct Pair {\n\tu8 first;\n\tu32 second;\n}\n%%end",
    );
    let main_source = module_source("src/main.w");
    let main_program = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\nchild.Pair item = { .first = 1; .second = 2; };\nunsafe { *?child.Pair pointer = &?item; *?u32 address = &?(*pointer).second; };\n%%end",
        );
    let imported_id = child_program.structs[0].id.clone();
    let namespace = match &main_program.items[0] {
        ScalarItem::Namespace(namespace) => (namespace.binding.clone(), namespace.span),
        _ => panic!("namespace item"),
    };
    let project = ScalarProject::new(
        vec![
            ScalarModule::from_program(
                main_program,
                vec![ScalarNamespaceBinding {
                    binding: namespace.0,
                    target: child_source.clone(),
                    span: namespace.1,
                }],
            ),
            ScalarModule::from_program(child_program, Vec::new()),
        ],
        vec![main_source.clone(), child_source],
    );
    let validation = validate_scalar_project(project);
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let ScalarItem::Binding(binding) = &validation.project.modules[0].items[1] else {
        panic!("struct binding")
    };
    assert_eq!(
        binding.declared_type,
        ScalarType::Struct(imported_id.clone())
    );
    let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
        &validation.project.modules[0].items[2]
    else {
        panic!("unsafe block")
    };
    let ScalarBlockItem::LocalBinding(binding) = &block.items[1] else {
        panic!("address binding")
    };
    let ScalarExpression::RawAddress {
        place: ScalarPlace::Field { field, .. },
        ..
    } = &binding.value
    else {
        panic!("field address")
    };
    assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == imported_id));
}

#[test]
fn types_imported_struct_field_checked_addresses_in_projects() {
    let child_source = module_source("src/child.w");
    let child_program = module_from_text(
        child_source.clone(),
        "%%start\nstruct Pair {\n\tu8 first;\n\tu32 second;\n}\n%%end",
    );
    let imported_id = child_program.structs[0].id.clone();
    let main_source = module_source("src/main.w");
    let main_program = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\nchild.Pair item = { .first = 1; .second = 2; };\n*u32 address = &item.second;\n*child.Pair pair = &item;\nu32 read = (*pair).second;\n%%end",
        );
    let namespace_span = match &main_program.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                main_program,
                vec![ScalarNamespaceBinding {
                    binding: "child".to_owned(),
                    target: child_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::from_program(child_program, Vec::new()),
        ],
        vec![main_source, child_source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let ScalarItem::Binding(binding) = &validation.project.modules[0].items[2] else {
        panic!("checked address binding")
    };
    assert_eq!(
        binding.declared_type,
        ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Shared,
            inner: Box::new(ScalarType::U32),
        }
    );
    let ScalarExpression::CheckedAddress {
        place: ScalarPlace::Field { field, .. },
        ..
    } = &binding.value
    else {
        panic!("checked field address")
    };
    assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == imported_id));
}

#[test]
fn resolves_imported_struct_fixed_array_field_indexes_and_addresses_in_projects() {
    let child_source = module_source("src/child.w");
    let child_program = module_from_text(
        child_source.clone(),
        "%%start\nstruct Record {\n\tu8[3] bytes;\n}\n%%end",
    );
    let imported_id = child_program.structs[0].id.clone();
    let main_source = module_source("src/main.w");
    let main_text = "%%start\nchild = namespace app \"src/child.w\";\nchild.Record value = { .bytes = [3, 5, 7]; };\nu64 index = 1;\nu8 result = value.bytes[index];\n*u8 shared = &value.bytes[index];\n*!u8 mutable = &!value.bytes[index];\nunsafe { *?u8 raw = &?value.bytes[index]; };\n%%end";
    let main_program = module_from_text(main_source.clone(), main_text);
    let namespace_span = match &main_program.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                main_program,
                vec![ScalarNamespaceBinding {
                    binding: "child".to_owned(),
                    target: child_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::from_program(child_program, Vec::new()),
        ],
        vec![main_source, child_source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );

    let ScalarItem::Binding(value) = &validation.project.modules[0].items[1] else {
        panic!("imported struct binding")
    };
    assert_eq!(value.declared_type, ScalarType::Struct(imported_id.clone()));
    let field_starts = main_text
        .match_indices("value.bytes[index]")
        .map(|(start, _)| start as u32)
        .collect::<Vec<_>>();
    let ScalarItem::Binding(result) = &validation.project.modules[0].items[3] else {
        panic!("indexed read binding")
    };
    assert_eq!(result.declared_type, ScalarType::U8);
    let ScalarExpression::IndexedRead {
        place:
            ScalarPlace::Index {
                base,
                span,
                index_span,
                ..
            },
        ..
    } = &result.value
    else {
        panic!("indexed read")
    };
    let expected_field_span = ByteSpan::new(field_starts[0], field_starts[0] + 11);
    let expected_index_span = ByteSpan::new(field_starts[0] + 12, field_starts[0] + 17);
    assert_eq!(
        *span,
        ByteSpan::new(expected_field_span.start, field_starts[0] + 18)
    );
    assert_eq!(*index_span, expected_index_span);
    let ScalarPlace::Field { field, span, .. } = base.as_ref() else {
        panic!("imported fixed-array field")
    };
    assert_eq!(*span, expected_field_span);
    assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == imported_id));

    for (address_index, (item_index, expected_type)) in [
        (
            4,
            ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Shared,
                inner: Box::new(ScalarType::U8),
            },
        ),
        (
            5,
            ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Mutable,
                inner: Box::new(ScalarType::U8),
            },
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let ScalarItem::Binding(binding) = &validation.project.modules[0].items[item_index] else {
            panic!("indexed checked address binding")
        };
        assert_eq!(binding.declared_type, expected_type);
        let ScalarExpression::CheckedAddress {
            place: ScalarPlace::Index {
                base, index_span, ..
            },
            ..
        } = &binding.value
        else {
            panic!("indexed checked address")
        };
        let field_start = field_starts[address_index + 1];
        let expected_field_span = ByteSpan::new(field_start, field_start + 11);
        assert_eq!(
            *index_span,
            ByteSpan::new(field_start + 12, field_start + 17)
        );
        let ScalarPlace::Field { field, span, .. } = base.as_ref() else {
            panic!("imported fixed-array field")
        };
        assert_eq!(*span, expected_field_span);
        assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == imported_id));
    }

    let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
        &validation.project.modules[0].items[6]
    else {
        panic!("unsafe block")
    };
    let ScalarBlockItem::LocalBinding(raw) = &block.items[0] else {
        panic!("indexed raw address binding")
    };
    assert_eq!(
        raw.declared_type,
        ScalarType::RawPointer(Box::new(ScalarType::U8))
    );
    let ScalarExpression::RawAddress {
        place: ScalarPlace::Index {
            base, index_span, ..
        },
        ..
    } = &raw.value
    else {
        panic!("indexed raw address")
    };
    let field_start = field_starts[3];
    let expected_field_span = ByteSpan::new(field_start, field_start + 11);
    assert_eq!(
        *index_span,
        ByteSpan::new(field_start + 12, field_start + 17)
    );
    let ScalarPlace::Field { field, span, .. } = base.as_ref() else {
        panic!("imported fixed-array field")
    };
    assert_eq!(*span, expected_field_span);
    assert!(matches!(field, ScalarFieldReference::Resolved(id) if id.structure == imported_id));
}

#[test]
fn validates_imported_struct_fixed_array_field_writes_in_projects() {
    let child_source = module_source("src/child.w");
    let child_program = module_from_text(
        child_source.clone(),
        "%%start\nstruct Record { u8[3] bytes; }\n%%end",
    );
    let main_source = module_source("src/main.w");
    let main_program = module_from_text(
            main_source.clone(),
            "%%start\nchild = namespace app \"src/child.w\";\nchild.Record holder = { .bytes = [1, 2, 3]; };\nu64 index = 1;\nholder.bytes[0], holder.bytes[2] = 4, 5;\nholder.bytes[index] = 6;\n%%end",
        );
    let namespace_span = match &main_program.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                main_program,
                vec![ScalarNamespaceBinding {
                    binding: "child".to_owned(),
                    target: child_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::from_program(child_program, Vec::new()),
        ],
        vec![main_source, child_source],
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
}

#[test]
fn resolves_utf8_literals_through_aliased_namespace_targets() {
    let std_source = module_source("src/bootstrap.w");
    let std = module_from_text(
        std_source.clone(),
        "%%start\nstruct utf8 {\n\t*?u8 data;\n\tu64 length;\n}\ni32 value = 80;\n%%end",
    );
    let imported_id = std.structs[0].id.clone();
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\ntext = namespace std \"src/bootstrap.w\";\nstd = namespace std \"src/bootstrap.w\";\ni32 observed = text.value;\nstd.utf8 standard = \"\";\ntext.utf8 literal = \"hé\";\n*?u8 data = literal.data;\nu64 length = literal.length;\nunsafe { *?text.utf8 pointer = &?literal; *?u8 pointer_data = pointer.data; u64 pointer_length = pointer.length; };\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::new(
                main_source,
                main.items,
                vec![
                    ScalarNamespaceBinding {
                        binding: "text".to_owned(),
                        target: std_source.clone(),
                        span: namespace_span,
                    },
                    ScalarNamespaceBinding {
                        binding: "std".to_owned(),
                        target: std_source.clone(),
                        span: namespace_span,
                    },
                ],
            ),
            ScalarModule::from_program(std, Vec::new()),
        ],
        Vec::new(),
    ));
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let ScalarItem::Binding(observed) = &validation.project.modules[0].items[2] else {
        panic!("namespace member binding")
    };
    assert!(matches!(
        observed.value,
        ScalarExpression::Member { ref receiver, ref name, .. }
            if receiver == "text" && name == "value"
    ));
    let ScalarItem::Binding(standard) = &validation.project.modules[0].items[3] else {
        panic!("conventional utf8 binding")
    };
    assert_eq!(
        standard.declared_type,
        ScalarType::Struct(imported_id.clone())
    );
    let ScalarItem::Binding(text) = &validation.project.modules[0].items[4] else {
        panic!("utf8 binding")
    };
    assert_eq!(text.declared_type, ScalarType::Struct(imported_id.clone()));
    assert!(matches!(
        text.value,
        ScalarExpression::Utf8 { ref value, .. } if value == b"h\xc3\xa9"
    ));
    let ScalarItem::Binding(data) = &validation.project.modules[0].items[5] else {
        panic!("data binding")
    };
    assert_eq!(
        data.declared_type,
        ScalarType::RawPointer(Box::new(ScalarType::U8))
    );
    let ScalarItem::Binding(length) = &validation.project.modules[0].items[6] else {
        panic!("length binding")
    };
    assert_eq!(length.declared_type, ScalarType::U64);
}

#[test]
fn keeps_same_index_structs_distinct_by_declaring_source() {
    let first_source = module_source("src/first.w");
    let first = module_from_text(
        first_source.clone(),
        "%%start\nstruct Pair { u32 value; }\n%%end",
    );
    let second_source = module_source("src/second.w");
    let second = module_from_text(
        second_source.clone(),
        "%%start\nstruct Pair { u32 value; }\n%%end",
    );
    assert_eq!(first.structs[0].id.index, second.structs[0].id.index);
    assert_ne!(first.structs[0].id, second.structs[0].id);
    assert_eq!(first.structs[0].id.source, first_source);
    assert_eq!(second.structs[0].id.source, second_source);
}

#[test]
fn derives_the_complete_integer_surface_and_radix_values() {
    let program = module_from_text(
        module_source("src/integers.w"),
        r#"%%start
i8 a = 127;
i16 b = 0x7fff;
i32 c = 2_147_483_647;
i64 d = 0x7fff_ffff_ffff_ffff;
i128 e = 0x7fff_ffff_ffff_ffff_ffff_ffff_ffff_ffff;
u8 f = 255;
u16 g = 0xffff;
u32 h = 0xffff_ffff;
u64 i = 0xffff_ffff_ffff_ffff;
u128 j = 0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff;
%%end"#,
    );
    assert!(program.items.iter().all(|item| match item {
        ScalarItem::Binding(binding) => !matches!(binding.declared_type, ScalarType::Named { .. }),
        _ => true,
    }));
    assert!(program.items.iter().all(|item| match item {
        ScalarItem::Binding(binding) => {
            matches!(binding.value, ScalarExpression::Integer { .. })
        }
        _ => true,
    }));
}

#[test]
fn derives_contextual_finite_float_literals_across_scalar_positions() {
    let text = "%%start\nf32(f32) narrow = fn(value) { value };\nf64(f64) wide = fn(value) { value };\nenv = extern wasm \"env\" { unit(f32, f64) consume; };\nf32 first = 1_2.5e-1;\nf64 second = 2E+3;\nf32 sum = first + 2.5;\nbool ordered = second < 3e3;\nf32 echoed = narrow(4e-1);\nf64 widened = wide(5.0);\nenv.consume(6.0, 7e1);\n%%end";
    let result = validate_text(text);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Binding(first) = &result.program.items[3] else {
        panic!("first float binding")
    };
    let ScalarExpression::Float { spelling, span, .. } = &first.value else {
        panic!("first float literal")
    };
    assert_eq!(spelling, "1_2.5e-1");
    let start = text.find(spelling).expect("float literal") as u32;
    assert_eq!(*span, ByteSpan::new(start, start + spelling.len() as u32));
    assert_eq!(first.declared_type, ScalarType::F32);
    let ScalarItem::Binding(second) = &result.program.items[4] else {
        panic!("second float binding")
    };
    assert_eq!(second.declared_type, ScalarType::F64);
}

#[test]
fn rejects_invalid_and_out_of_range_float_literals_at_literal_spans() {
    for literal in ["1__2.0", "1._2", "1e_2", "1e400"] {
        let text = format!("%%start\nf64 value = {literal};\n%%end");
        let result = validate_text(&text);
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "B0010")
            .expect("float diagnostic");
        let start = text.find(literal).expect("float literal") as u32;
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + literal.len() as u32)
        );
    }
}

#[test]
fn rejects_float_width_and_integer_float_mismatches() {
    for text in [
        "%%start\nf32 narrow = 1.0;\nf64 wide = narrow;\n%%end",
        "%%start\nf64 wide = 1.0;\nf32 narrow = wide;\n%%end",
        "%%start\ni32 integer = 1;\nf32 decimal = integer;\n%%end",
        "%%start\nf64 decimal = 1.0;\ni32 integer = decimal;\n%%end",
    ] {
        let result = validate_text(text);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "B0003"),
            "{text}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn derives_local_pair_returns_in_source_order() {
    let result =
        validate_text("%%start\n(i32, bool)() pair = fn { i32 local = 1; local, true };\n%%end");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Function(function) = &result.program.items[0] else {
        panic!("pair function")
    };
    assert_eq!(function.body.final_output_values.len(), 2);
    assert_eq!(function.body.final_output_values[0].position, 0);
    assert_eq!(function.body.final_output_values[0].ty, ScalarType::I32);
    assert_eq!(function.body.final_output_values[1].position, 1);
    assert_eq!(function.body.final_output_values[1].ty, ScalarType::Bool);
    assert!(matches!(
        &function.body.final_output_values[0].value,
        ScalarExpression::Name { ref name, .. } if name == "local"
    ));
    assert!(matches!(
        &function.body.final_output_values[1].value,
        ScalarExpression::Boolean { value: true, .. }
    ));
}

#[test]
fn derives_conditional_output_lists_by_position() {
    let result = validate_text(
        "%%start\n(i32, bool)() pair = fn { if (true) { 1, true } else { 2, false } };\n%%end",
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Function(function) = &result.program.items[0] else {
        panic!("conditional pair function")
    };
    assert_eq!(function.body.final_output_values.len(), 2);
    assert_eq!(function.body.final_output_values[0].ty, ScalarType::I32);
    assert_eq!(function.body.final_output_values[1].ty, ScalarType::Bool);
    assert!(matches!(
        &function.body.final_output_values[0].value,
        ScalarExpression::If { .. }
    ));
}

#[test]
fn validates_nested_conditional_output_lists_by_position() {
    let result = validate_text(
            "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, false } } else { 3, true } };\n%%end",
        );

    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
}

#[test]
fn validates_nested_conditional_output_lists_in_a_project() {
    let source = module_source("src/pair.w");
    let program = module_from_text(
            source.clone(),
            "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, false } } else { 3, true } };\n%%end",
        );
    let validation = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(program, Vec::new())],
        vec![source],
    ));

    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
}

#[test]
fn reports_nested_conditional_output_type_at_the_inner_branch() {
    let text = "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, 3 } } else { 3, true } };\n%%end";
    let validation = validate_text(text);
    let diagnostic = validation
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0006")
        .expect("inner branch type diagnostic");
    let start = text
        .find("if (true) { 1, true } else { 2, 3 }")
        .expect("inner conditional");

    assert_eq!(
        diagnostic.labels[0].span.range,
        ByteSpan::new(
            start as u32,
            (start + "if (true) { 1, true } else { 2, 3 }".len()) as u32
        )
    );
}

#[test]
fn reports_nested_conditional_output_arity_at_the_inner_branch() {
    let text = "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1 } else { 2, false } } else { 3, true } };\n%%end";
    let validation = validate_text(text);
    let diagnostic = validation
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "B0004")
        .expect("inner branch arity diagnostic");
    let start = text.find("{ 1 }").expect("inner branch") + 2;

    assert_eq!(
        diagnostic.labels[0].span.range,
        ByteSpan::new(start as u32, start as u32 + 1)
    );
}

#[test]
fn reports_final_output_list_type_and_arity_diagnostics() {
    let wrong_type = validate_text("%%start\n(i32, bool)() pair = fn { true, 1 };\n%%end");
    assert!(wrong_type
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0003"));

    let wrong_arity = validate_text("%%start\n(i32, bool)() pair = fn { 1, true, false };\n%%end");
    assert!(wrong_arity.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "B0004"
            && diagnostic.message == "function output arity does not match its final output list"
    }));
}

#[test]
fn validates_control_flow_prefix_before_final_output_list_as_unit() {
    let text = "%%start\n(i32, bool)() pair = fn { if (true) { 1; } else { 2; } while (false) { 3; } 7, true };\n%%end";
    let result = validate_text(text);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

    let source = source();
    let program = module_from_text(source.clone(), text);
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), program.items, Vec::new())],
        vec![source],
    ));
    assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
}

#[test]
fn preserves_final_output_expression_spans() {
    let text = "%%start\n(i32, bool)() pair = fn { 12, false };\n%%end";
    let result = validate_text(text);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Function(function) = &result.program.items[0] else {
        panic!("pair function")
    };
    let first_start = text.find("12").expect("first output") as u32;
    let second_start = text.find("false").expect("second output") as u32;
    assert_eq!(
        function.body.final_output_values[0].span,
        ByteSpan::new(first_start, first_start + 2)
    );
    assert_eq!(
        function.body.final_output_values[1].span,
        ByteSpan::new(second_start, second_start + 5)
    );
}

#[test]
fn derives_and_contextually_validates_fixed_array_literals() {
    let valid = validate_text("%%start\nu8[2] bytes = [1, 2];\n%%end");
    assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
    let ScalarItem::Binding(binding) = &valid.program.items[0] else {
        panic!("array binding")
    };
    assert!(matches!(
        binding.declared_type,
        ScalarType::Array { length: 2, .. }
    ));
    assert!(matches!(
        binding.value,
        ScalarExpression::ArrayLiteral { .. }
    ));

    let count = validate_text("%%start\nu8[2] bytes = [1];\n%%end");
    assert!(count.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "array literal element count does not match fixed array length"
    }));
    let element = validate_text("%%start\nu8[1] bytes = [300];\n%%end");
    assert!(element
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0010"));
}

#[test]
fn rejects_invalid_fixed_array_lengths_at_the_complete_length_span() {
    for length in ["-1", "18446744073709551616"] {
        let text = format!("%%start\nu8[{length}] value = [0];\n%%end");
        let validation = validate_text(&text);
        assert_eq!(
            validation.diagnostics.len(),
            1,
            "{:?}",
            validation.diagnostics
        );
        let diagnostic = &validation.diagnostics[0];
        assert_eq!(diagnostic.code, "B0003");
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(
                text.find(length).expect("length") as u32,
                (text.find(length).expect("length") + length.len()) as u32,
            )
        );
        let ScalarItem::Binding(binding) = &validation.program.items[0] else {
            panic!("array binding")
        };
        assert_eq!(binding.declared_type, ScalarType::Error);
    }
}

#[test]
fn fixed_array_layout_overflow_is_a_single_diagnostic_not_a_panic() {
    let text =
        "%%start\nstruct Huge {\n\tu128[18446744073709551615] values;\n\tu32 later;\n}\n%%end";
    let validation = validate_text(text);
    assert_eq!(
        validation.diagnostics.len(),
        1,
        "{:?}",
        validation.diagnostics
    );
    assert_eq!(validation.diagnostics[0].code, "B0003");
    let length = "18446744073709551615";
    assert_eq!(
        validation.diagnostics[0].labels[0].span.range,
        ByteSpan::new(
            text.find(length).expect("length") as u32,
            (text.find(length).expect("length") + length.len()) as u32,
        )
    );
    let huge = &validation.program.structs[0];
    assert_eq!(huge.layout, None);
    assert_eq!(huge.fields[0].layout, None);
    assert_eq!(huge.fields[0].offset, None);
    assert_eq!(huge.fields[1].layout, None);
    assert_eq!(huge.fields[1].offset, None);
    assert!(crate::emit_scalar_llvm(&validation).is_err());

    let module = ScalarModule::from_program(validation.program, Vec::new());
    let project = validate_scalar_project(ScalarProject::new(vec![module], vec![source()]));
    assert_eq!(project.diagnostics.len(), 1, "{:?}", project.diagnostics);
    assert_eq!(project.diagnostics[0].code, "B0003");
}

#[test]
fn aggregate_layout_addition_overflow_is_a_single_diagnostic_with_absent_layouts() {
    let text = "%%start\nstruct Huge {\n\tu8[18446744073709551615] values;\n\tu8 later;\n}\n%%end";
    let validation = validate_text(text);
    assert_eq!(
        validation.diagnostics.len(),
        1,
        "{:?}",
        validation.diagnostics
    );
    let diagnostic = &validation.diagnostics[0];
    assert_eq!(diagnostic.code, "B0003");
    assert_eq!(diagnostic.message, "struct layout exceeds u64");
    let later = "later";
    assert_eq!(
        diagnostic.labels[0].span.range,
        ByteSpan::new(
            text.find(later).expect("later field") as u32,
            (text.find(later).expect("later field") + later.len()) as u32,
        )
    );
    let huge = &validation.program.structs[0];
    assert_eq!(huge.layout, None);
    assert!(huge.fields.iter().all(|field| field.layout.is_none()));
    assert!(huge.fields.iter().all(|field| field.offset.is_none()));
    assert!(crate::emit_scalar_llvm(&validation).is_err());

    let source = source();
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::from_program(
            module_from_text(source.clone(), text),
            Vec::new(),
        )],
        vec![source],
    ));
    assert_eq!(project.diagnostics.len(), 1, "{:?}", project.diagnostics);
    assert_eq!(project.diagnostics[0].code, "B0003");
    assert_eq!(project.diagnostics[0].message, "struct layout exceeds u64");
    let huge = &project.project.modules[0].structs[0];
    assert_eq!(huge.layout, None);
    assert!(huge.fields.iter().all(|field| field.layout.is_none()));
    assert!(huge.fields.iter().all(|field| field.offset.is_none()));
    assert!(crate::emit_scalar_project_llvm(&project).is_err());
}

#[test]
fn rejects_direct_extern_fixed_array_inputs_and_outputs() {
    let text =
        "%%start\nenv = extern wasm \"env\" { unit(u8[2]) consume; u8[2]() produce; };\n%%end";
    let validation = validate_text(text);
    let diagnostics = validation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message == "direct FFI arrays are not supported")
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 2, "{:?}", validation.diagnostics);
    assert!(diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code == "B0003"));
}

#[test]
fn resolves_transitive_local_aggregate_layout_absence() {
    let text = "%%start
struct Outer {
	Middle middle;
}
struct Middle {
	Huge huge;
}
struct Huge {
	u8[18446744073709551615] values;
	u8 later;
}
%%end";
    let validation = validate_text(text);
    assert_eq!(
        validation.diagnostics.len(),
        1,
        "{:?}",
        validation.diagnostics
    );
    assert_eq!(
        validation.diagnostics[0].message,
        "struct layout exceeds u64"
    );
    assert_eq!(
        validation.diagnostics[0].labels[0].span.range,
        ByteSpan::new(
            text.rfind("later").expect("later field") as u32,
            (text.rfind("later").expect("later field") + "later".len()) as u32,
        )
    );
    assert!(validation.program.structs.iter().all(|structure| {
        structure.layout.is_none()
            && structure
                .fields
                .iter()
                .all(|field| field.layout.is_none() && field.offset.is_none())
    }));
    assert!(crate::emit_scalar_llvm(&validation).is_err());
}

#[test]
fn resolves_transitive_project_aggregate_layout_absence() {
    let huge_source = module_source("src/huge.w");
    let huge = module_from_text(
        huge_source.clone(),
        "%%start
struct Huge {
	u8[18446744073709551615] values;
	u8 later;
}
%%end",
    );
    let middle_source = module_source("src/middle.w");
    let middle = module_from_text(
        middle_source.clone(),
        "%%start
huge = namespace app \"src/huge.w\";
struct Middle {
	huge.Huge value;
}
%%end",
    );
    let outer_source = module_source("src/main.w");
    let outer = module_from_text(
        outer_source.clone(),
        "%%start
middle = namespace app \"src/middle.w\";
struct Outer {
	middle.Middle value;
}
%%end",
    );
    let outer_namespace_span = match &outer.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("outer namespace"),
    };
    let middle_namespace_span = match &middle.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("middle namespace"),
    };
    let project = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                outer,
                vec![ScalarNamespaceBinding {
                    binding: "middle".to_owned(),
                    target: middle_source.clone(),
                    span: outer_namespace_span,
                }],
            ),
            ScalarModule::from_program(
                middle,
                vec![ScalarNamespaceBinding {
                    binding: "huge".to_owned(),
                    target: huge_source.clone(),
                    span: middle_namespace_span,
                }],
            ),
            ScalarModule::from_program(huge, Vec::new()),
        ],
        vec![outer_source, middle_source, huge_source],
    ));
    assert_eq!(project.diagnostics.len(), 1, "{:?}", project.diagnostics);
    assert_eq!(project.diagnostics[0].message, "struct layout exceeds u64");
    assert!(project.project.modules.iter().all(|module| {
        module.structs.iter().all(|structure| {
            structure.layout.is_none()
                && structure
                    .fields
                    .iter()
                    .all(|field| field.layout.is_none() && field.offset.is_none())
        })
    }));
    assert!(crate::emit_scalar_project_llvm(&project).is_err());
}

#[test]
fn rejects_recursive_by_value_struct_layouts_once_per_component() {
    for text in [
        "%%start
struct Node {
	Node next;
}
%%end",
        "%%start
struct First {
	Second second;
}
struct Second {
	First first;
}
%%end",
    ] {
        let validation = validate_text(text);
        assert_eq!(
            validation.diagnostics.len(),
            1,
            "{:?}",
            validation.diagnostics
        );
        assert_eq!(
            validation.diagnostics[0].message,
            "recursive by-value struct layout is unsupported"
        );
        assert!(validation.program.structs.iter().all(|structure| {
            structure.layout.is_none()
                && structure
                    .fields
                    .iter()
                    .all(|field| field.layout.is_none() && field.offset.is_none())
        }));
    }
}

#[test]
fn rejects_three_member_recursive_layout_with_duplicate_participating_edges_once() {
    let text = "%%start
struct First {
	Second first_second;
	Second second_second;
}
struct Second {
	Third third;
}
struct Third {
	First first;
}
%%end";
    let validation = validate_text(text);
    let diagnostics = validation
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.message == "recursive by-value struct layout is unsupported"
        })
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1, "{:?}", validation.diagnostics);
    let first_field = "first_second";
    let start = text.find(first_field).expect("first participating field") as u32;
    assert_eq!(
        diagnostics[0].labels[0].span.range,
        ByteSpan::new(start, start + first_field.len() as u32)
    );
    assert!(validation.program.structs.iter().all(|structure| {
        structure.layout.is_none()
            && structure
                .fields
                .iter()
                .all(|field| field.layout.is_none() && field.offset.is_none())
    }));
}

#[test]
fn rejects_nested_array_recursive_layout_while_raw_pointers_and_callables_are_non_edges() {
    let text = "%%start
struct Node {
	*?Node parent;
	unit(Node) visit;
	Node[1][1] children;
}
%%end";
    let validation = validate_text(text);
    let diagnostics = validation
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.message == "recursive by-value struct layout is unsupported"
        })
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1, "{:?}", validation.diagnostics);
    let field = "children";
    let start = text.find(field).expect("recursive field") as u32;
    assert_eq!(
        diagnostics[0].labels[0].span.range,
        ByteSpan::new(start, start + field.len() as u32)
    );
    let node = &validation.program.structs[0];
    assert_eq!(node.layout, None);
    assert!(node
        .fields
        .iter()
        .all(|field| field.layout.is_none() && field.offset.is_none()));
}

#[test]
fn reports_recursive_and_overflowing_connected_aggregate_layouts_independently() {
    let text = "%%start
struct Root {
	Recursive recursive;
	Huge huge;
}
struct Recursive {
	Recursive member;
}
struct Huge {
	u8[18446744073709551615] values;
	u8 later;
}
%%end";
    let validation = validate_text(text);
    let diagnostics = validation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "B0003")
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 2, "{:?}", validation.diagnostics);
    assert_eq!(
        diagnostics[0].message,
        "recursive by-value struct layout is unsupported"
    );
    assert_eq!(diagnostics[1].message, "struct layout exceeds u64");
    for (diagnostic, field) in diagnostics.iter().zip(["member", "later"]) {
        let start = text.rfind(field).expect("diagnostic field") as u32;
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + field.len() as u32)
        );
    }
    assert!(validation.program.structs.iter().all(|structure| {
        structure.layout.is_none()
            && structure
                .fields
                .iter()
                .all(|field| field.layout.is_none() && field.offset.is_none())
    }));
    assert!(crate::emit_scalar_llvm(&validation).is_err());
}

#[test]
fn reports_local_recursive_layout_components_in_source_order() {
    let text = "%%start
struct Root {
	Late late;
}
struct Early {
	Early early;
}
struct Late {
	Late late;
}
%%end";
    let validation = validate_text(text);
    let diagnostics = validation
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.message == "recursive by-value struct layout is unsupported"
        })
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 2, "{:?}", validation.diagnostics);
    for (diagnostic, field) in diagnostics.iter().zip(["early", "late"]) {
        let start = text.rfind(field).expect("recursive field") as u32;
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + field.len() as u32)
        );
    }
    for structure in &validation.program.structs[1..] {
        assert_eq!(structure.layout, None);
        assert!(structure
            .fields
            .iter()
            .all(|field| { field.layout.is_none() && field.offset.is_none() }));
    }
}

#[test]
fn reports_project_recursive_layout_components_in_source_order() {
    let main_source = module_source("src/main.w");
    let main_text = "%%start
late = namespace app \"src/late.w\";
struct Root {
	late.Late late;
}
struct Early {
	Early early;
}
%%end";
    let main = module_from_text(main_source.clone(), main_text);
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("late namespace"),
    };
    let late_source = module_source("src/late.w");
    let late_text = "%%start
struct Late {
	Late late;
}
%%end";
    let late = module_from_text(late_source.clone(), late_text);
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                main,
                vec![ScalarNamespaceBinding {
                    binding: "late".to_owned(),
                    target: late_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::from_program(late, Vec::new()),
        ],
        vec![main_source.clone(), late_source.clone()],
    ));
    let diagnostics = validation
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.message == "recursive by-value struct layout is unsupported"
        })
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 2, "{:?}", validation.diagnostics);
    for (diagnostic, (source, text, field)) in diagnostics.iter().zip([
        (&main_source, main_text, "early"),
        (&late_source, late_text, "late"),
    ]) {
        let start = text.find(field).expect("recursive field") as u32;
        assert_eq!(diagnostic.labels[0].span.source, *source);
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + field.len() as u32)
        );
    }
    for structure in [
        &validation.project.modules[0].structs[1],
        &validation.project.modules[1].structs[0],
    ] {
        assert_eq!(structure.layout, None);
        assert!(structure
            .fields
            .iter()
            .all(|field| { field.layout.is_none() && field.offset.is_none() }));
    }
}

#[test]
fn reports_sibling_import_recursive_layout_components_in_import_observation_order() {
    let root_source = module_source("src/main.w");
    let root = module_from_text(
        root_source.clone(),
        "%%start
zeta = namespace app \"src/zeta.w\";
alpha = namespace app \"src/alpha.w\";
%%end",
    );
    let zeta_source = module_source("src/zeta.w");
    let zeta_text = "%%start
struct Zeta {
	Zeta zeta;
}
%%end";
    let zeta = module_from_text(zeta_source.clone(), zeta_text);
    let alpha_source = module_source("src/alpha.w");
    let alpha_text = "%%start
struct Alpha {
	Alpha alpha;
}
%%end";
    let alpha = module_from_text(alpha_source.clone(), alpha_text);
    let zeta_namespace = match &root.items[0] {
        ScalarItem::Namespace(namespace) => (namespace.binding.clone(), namespace.span),
        _ => panic!("zeta namespace"),
    };
    let alpha_namespace = match &root.items[1] {
        ScalarItem::Namespace(namespace) => (namespace.binding.clone(), namespace.span),
        _ => panic!("alpha namespace"),
    };
    let validation = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                root,
                vec![
                    ScalarNamespaceBinding {
                        binding: zeta_namespace.0,
                        target: zeta_source.clone(),
                        span: zeta_namespace.1,
                    },
                    ScalarNamespaceBinding {
                        binding: alpha_namespace.0,
                        target: alpha_source.clone(),
                        span: alpha_namespace.1,
                    },
                ],
            ),
            ScalarModule::from_program(zeta, Vec::new()),
            ScalarModule::from_program(alpha, Vec::new()),
        ],
        vec![root_source, zeta_source.clone(), alpha_source.clone()],
    ));
    let diagnostics = validation
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == "B0003"
                && diagnostic.message == "recursive by-value struct layout is unsupported"
        })
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 2, "{:?}", validation.diagnostics);
    for (diagnostic, (source, text, field)) in diagnostics.iter().zip([
        (&zeta_source, zeta_text, "zeta"),
        (&alpha_source, alpha_text, "alpha"),
    ]) {
        let start = text.rfind(field).expect("recursive field") as u32;
        assert_eq!(diagnostic.labels[0].span.source, *source);
        assert_eq!(
            diagnostic.labels[0].span.range,
            ByteSpan::new(start, start + field.len() as u32)
        );
    }
    for module in &validation.project.modules[1..] {
        assert!(module.structs.iter().all(|structure| {
            structure.layout.is_none()
                && structure
                    .fields
                    .iter()
                    .all(|field| field.layout.is_none() && field.offset.is_none())
        }));
    }
}

#[test]
fn absent_local_struct_layout_stops_containing_layout_accumulation() {
    let text = "%%start\nstruct Outer {\n\tHuge huge;\n\tu32 later;\n}\nstruct Huge {\n\tu128[18446744073709551615] values;\n}\n%%end";
    let validation = validate_text(text);
    assert_eq!(
        validation.diagnostics.len(),
        1,
        "{:?}",
        validation.diagnostics
    );
    assert_eq!(validation.diagnostics[0].code, "B0003");
    let length = "18446744073709551615";
    assert_eq!(
        validation.diagnostics[0].labels[0].span.range,
        ByteSpan::new(
            text.find(length).expect("length") as u32,
            (text.find(length).expect("length") + length.len()) as u32,
        )
    );
    let outer = &validation.program.structs[0];
    assert_eq!(outer.layout, None);
    assert_eq!(outer.fields[0].layout, None);
    assert_eq!(outer.fields[0].offset, None);
    assert_eq!(outer.fields[1].layout, None);
    assert_eq!(outer.fields[1].offset, None);
    let huge = &validation.program.structs[1];
    assert_eq!(huge.layout, None);
    assert_eq!(huge.fields[0].layout, None);
    assert_eq!(huge.fields[0].offset, None);
}

#[test]
fn absent_project_struct_layout_propagates_without_a_duplicate_diagnostic() {
    let library_source = module_source("src/library.w");
    let library_text = "%%start\nstruct Huge {\n\tu128[18446744073709551615] values;\n}\n%%end";
    let library = module_from_text(library_source.clone(), library_text);
    let main_source = module_source("src/main.w");
    let main = module_from_text(
            main_source.clone(),
            "%%start\nlibrary = namespace app \"src/library.w\";\nstruct Outer {\n\tlibrary.Huge huge;\n\tu32 later;\n}\n%%end",
        );
    let namespace_span = match &main.items[0] {
        ScalarItem::Namespace(namespace) => namespace.span,
        _ => panic!("namespace item"),
    };
    let project = validate_scalar_project(ScalarProject::new(
        vec![
            ScalarModule::from_program(
                main,
                vec![ScalarNamespaceBinding {
                    binding: "library".to_owned(),
                    target: library_source.clone(),
                    span: namespace_span,
                }],
            ),
            ScalarModule::from_program(library, Vec::new()),
        ],
        vec![main_source, library_source],
    ));
    assert_eq!(project.diagnostics.len(), 1, "{:?}", project.diagnostics);
    assert_eq!(project.diagnostics[0].code, "B0003");
    let length = "18446744073709551615";
    assert_eq!(
        project.diagnostics[0].labels[0].span.range,
        ByteSpan::new(
            library_text.find(length).expect("length") as u32,
            (library_text.find(length).expect("length") + length.len()) as u32,
        )
    );
    let outer = &project.project.modules[0].structs[0];
    assert_eq!(outer.layout, None);
    assert_eq!(outer.fields[0].layout, None);
    assert_eq!(outer.fields[0].offset, None);
    assert_eq!(outer.fields[1].layout, None);
    assert_eq!(outer.fields[1].offset, None);
    let huge = &project.project.modules[1].structs[0];
    assert_eq!(huge.layout, None);
    assert_eq!(huge.fields[0].layout, None);
    assert_eq!(huge.fields[0].offset, None);
    assert!(crate::emit_scalar_project_llvm(&project).is_err());
}

#[test]
fn error_type_is_not_a_valid_equal_type() {
    assert!(!scalar_type_equal(&ScalarType::Error, &ScalarType::Error));
    assert!(!scalar_type_equal(&ScalarType::Error, &ScalarType::U8));
}

#[test]
fn invalid_fixed_array_length_is_rejected_before_llvm_emission() {
    let validation = validate_text("%%start\nu8[-1] value = [0];\n%%end");
    assert!(crate::emit_scalar_llvm(&validation).is_err());
}

#[test]
fn validates_scoped_fixed_array_callable_transport_and_preserves_module_collision() {
    let valid = validate_text(
            "%%start\nu8[2](u8[2]) transport = fn(bytes) { bytes };\nunit(u8) observe = fn(value) { value; };\nunit() run = fn { u8[2] bytes = [7, 9]; transport(bytes); observe(1); };\n%%end",
        );
    assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
    let ScalarItem::Function(transport) = &valid.program.items[0] else {
        panic!("transport function")
    };
    let ScalarType::Callable {
        outputs,
        parameters,
    } = &transport.signature
    else {
        panic!("transport callable signature")
    };
    assert_eq!(outputs.outputs.len(), 1);
    assert_eq!(parameters.len(), 1);
    assert!(matches!(
        &outputs.outputs[0].ty,
        ScalarType::Array {
            element,
            length: 2,
            ..
        } if **element == ScalarType::U8
    ));
    assert!(matches!(
        &parameters[0],
        ScalarType::Array {
            element,
            length: 2,
            ..
        } if **element == ScalarType::U8
    ));

    let source = source();
    let invalid = module_from_text(
        source.clone(),
        "%%start\nu8[2] bytes = [1, 2];\nu8[2](u8[2]) transport = fn(bytes) { bytes };\n%%end",
    );
    let project = validate_scalar_project(ScalarProject::new(
        vec![ScalarModule::new(source.clone(), invalid.items, Vec::new())],
        vec![source],
    ));
    assert!(
        project
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0002"),
        "{:?}",
        project.diagnostics
    );
}

use super::*;
use crate::parse_source;

const CROSSED: &str = "%%start\nunit((u8())[]) inspect = fn(items) { items; };\nunit(bool) exercise = fn(flag) {\n\tu64 count = 2;\n\tu8() a = fn { 1 };\n\tu8() b = fn { 2 };\n\tu8() first = a;\n\tu8() second = b;\n\t(u8())[count] items;\n\titems[0] = fn { 9 };\n\titems[1] = fn { 10 };\n\tif (flag) {\n\t\tfirst = b;\n\t\tsecond = a;\n\t};\n\t*!(u8()) left = &!items[0];\n\t*!(u8()) right = &!items[1];\n\t*left = first;\n\t*right = second;\n\tinspect(items);\n};\nexercise(true);\nexercise(false);\n%%end";

const DIAGONAL: &str = "%%start\nunit((u8())[]) inspect = fn(items) { items; };\nunit(bool) exercise = fn(flag) {\n\tu64 count = 2;\n\tu8() a = fn { 1 };\n\tu8() b = fn { 2 };\n\tu8() first = a;\n\tu8() second = a;\n\t(u8())[count] items;\n\titems[0] = fn { 9 };\n\titems[1] = fn { 10 };\n\tif (flag) {\n\t\tfirst = b;\n\t\tsecond = b;\n\t};\n\t*!(u8()) left = &!items[0];\n\t*!(u8()) right = &!items[1];\n\t*left = first;\n\t*right = second;\n\tinspect(items);\n};\nexercise(true);\nexercise(false);\n%%end";

fn literal_producer(function: &ScalarFunction, name: &str) -> SourceSpan {
    let binding = function
        .body
        .items
        .iter()
        .find_map(|item| match item {
            ScalarBlockItem::LocalBinding(binding) if binding.name == name => Some(binding),
            _ => None,
        })
        .expect("actual canonical callable binding");
    let ScalarExpression::FunctionLiteral { span, .. } = &binding.value else {
        panic!("actual canonical callable literal")
    };
    SourceSpan::new(binding.occurrence.source_span.source.clone(), *span)
}

fn joint_tuples(
    backing: &RuntimeBackingFact,
    aliases: &[ReferencePlaceId; 2],
) -> Vec<(SourceSpan, SourceSpan)> {
    assert_eq!(backing.kind, RuntimeBackingKind::OwnedAllocation);
    assert_eq!(backing.callable_family.len(), 1);
    let family = &backing.callable_family[0];
    assert!(!family.partitions.is_empty());
    let mut tuples = Vec::new();
    for partition in &family.partitions {
        for case in partition
            .representatives
            .iter()
            .flat_map(|representative| &representative.cell.alternatives)
            .chain(&partition.remainder.alternatives)
        {
            if let CallableCellCaseFact::Initialized {
                backing: containing,
                owner,
                ..
            } = case
            {
                assert_eq!(containing, &backing.value);
                assert_eq!(owner, &backing.owner);
            }
        }
        let cells = [0, 1].map(|index| {
            let key = CallableIndexKeyFact::Constant(index.into());
            let representatives = partition
                .representatives
                .iter()
                .filter(|representative| representative.key == key)
                .collect::<Vec<_>>();
            assert_eq!(representatives.len(), 1);
            let cases = &representatives[0].cell.alternatives;
            assert_eq!(cases.len(), 1, "one whole cell outcome per joint partition");
            let CallableCellCaseFact::Initialized {
                environment_origin,
                holder_aliases,
                backing: containing,
                owner,
                origin,
                moved,
                ..
            } = &cases[0]
            else {
                panic!("actual initialized common pinned cell")
            };
            assert_eq!(containing, &backing.value);
            assert_eq!(owner, &backing.owner);
            assert_eq!(origin, &environment_origin.literal);
            assert!(!moved);
            assert_eq!(
                holder_aliases,
                &vec![CallableHolderPlace::Projection(
                    aliases[index as usize].clone()
                )]
            );
            environment_origin.literal.clone()
        });
        let [left, right] = cells;
        let tuple = (left, right);
        if !tuples.contains(&tuple) {
            tuples.push(tuple);
        }
    }
    tuples
}

fn assert_relation(
    actual: &[(SourceSpan, SourceSpan)],
    a: &SourceSpan,
    b: &SourceSpan,
    diagonal: bool,
) {
    assert_ne!(a, b);
    let expected = if diagonal {
        [(a.clone(), a.clone()), (b.clone(), b.clone())]
    } else {
        [(a.clone(), b.clone()), (b.clone(), a.clone())]
    };
    assert_eq!(actual.len(), 2, "exact whole contributor relation");
    for tuple in &expected {
        assert!(
            actual.contains(tuple),
            "missing actual joint tuple {tuple:?}"
        );
    }
    for (left, right) in actual {
        assert!(left == a || left == b);
        assert!(right == a || right == b);
        assert_eq!(left == right, diagonal, "spurious joint tuple");
    }
}

fn source_entry_correlation(layout: ScalarTargetLayout, project: bool, diagonal: bool) {
    let (text, path) = if diagonal {
        (DIAGONAL, "src/diagonal.w")
    } else {
        (CROSSED, "src/crossed.w")
    };
    let source = SourceIdentity::new(
        "family-join-proof".into(),
        "app".into(),
        path.into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let original_exercise = validation
        .program
        .items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "exercise" => Some(function),
            _ => None,
        })
        .expect("actual source exercise");
    let a = literal_producer(original_exercise, "a");
    let b = literal_producer(original_exercise, "b");
    let result = if project {
        let project = ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source.clone()],
        );
        let (validation, trace) = validate_scalar_project_observed_test(project);
        ObservedScalarValidation::Project { validation, trace }
    } else {
        ObservedScalarValidation::Program { validation, trace }
    };
    let (items, diagnostics, trace) = match &result {
        ObservedScalarValidation::Program { validation, trace } => {
            assert_eq!(validation.program.source, source);
            (&validation.program.items, &validation.diagnostics, trace)
        }
        ObservedScalarValidation::Project { validation, trace } => {
            assert_eq!(validation.project.modules.len(), 1);
            assert_eq!(validation.project.modules[0].source, source);
            (
                &validation.project.modules[0].items,
                &validation.diagnostics,
                trace,
            )
        }
    };
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let exercise = items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "exercise" => Some(function),
            _ => None,
        })
        .expect("actual validated exercise");
    let inspect = items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "inspect" => Some(function),
            _ => None,
        })
        .expect("actual validated inspect");
    assert_eq!(a, literal_producer(exercise, "a"));
    assert_eq!(b, literal_producer(exercise, "b"));
    assert_eq!(a.source, source);
    assert_eq!(b.source, source);
    let caller_cfg = &exercise.reference_cfg;
    let callee_cfg = &inspect.reference_cfg;
    for function in [exercise, inspect] {
        assert!(matches!(&function.reference_cfg.producer_owner,
            ProducerCfgOwnerId::Function { source: actual, declaration, body: ProducerFunctionBodyId::Declaration }
            if actual == &source && *declaration == function.name_span));
    }
    let caller = trace
        .passes
        .iter()
        .rfind(|pass| {
            pass.owner == caller_cfg.producer_owner
                && pass.role == ReferenceTransferTestRole::Function
        })
        .expect("final actual caller Function transfer pass");
    let callee = trace
        .passes
        .iter()
        .rfind(|pass| {
            pass.owner == callee_cfg.producer_owner
                && pass.role == ReferenceTransferTestRole::Function
        })
        .expect("final actual callee Function transfer pass");
    for (pass, cfg) in [(caller, caller_cfg), (callee, callee_cfg)] {
        assert!(pass.errors.is_empty(), "{:#?}", pass.errors);
        assert_eq!(pass.cfg_at_entry.producer_owner, cfg.producer_owner);
        assert_eq!(pass.cfg_at_exit, *cfg);
    }
    for literal in [&a, &b] {
        let environment = caller_cfg
            .environments
            .iter()
            .find(|environment| &environment.literal == literal)
            .expect("actual qualified literal environment producer");
        assert!(environment.capture_paths.is_empty());
        assert!(environment.loans.is_empty());
    }
    let items_binding = exercise
        .body
        .items
        .iter()
        .find_map(|item| match item {
            ScalarBlockItem::LocalBinding(binding) if binding.name == "items" => Some(binding),
            _ => None,
        })
        .expect("canonical source array declaration");
    let length_site = caller_cfg
        .backing_length_sites
        .iter()
        .find(|site| site.binding.declaration_span == items_binding.name_span)
        .expect("actual source array backing length site");
    let local_owner = &length_site.binding;
    assert_eq!(local_owner.source, source);
    assert_eq!(local_owner.function_span, exercise.span);
    assert_eq!(local_owner.declaration_span, items_binding.name_span);
    assert_eq!(local_owner.kind, ReferenceBindingKind::Declared);
    let creation = caller_cfg
        .initial_points
        .iter()
        .find(|point| {
            matches!(&point.kind, CfgPointKind::Backing {
            target, transfer: RuntimeBackingTransfer::Allocate, ..
        } if target == local_owner)
        })
        .expect("actual runtime array allocation producer");
    assert_eq!(creation.source_span.source, source);
    let initial_literals = [0, 1].map(|index| {
        let assignment = exercise.body.items.iter().find_map(|item| match item {
            ScalarBlockItem::Assignment(assignment)
                if assignment.targets.len() == 1 && matches!(&assignment.targets[0].place,
                    ScalarPlace::Index { base, index: selected, .. }
                        if matches!(base.as_ref(), ScalarPlace::Name { name, .. } if name == "items")
                        && matches!(selected.as_ref(), ScalarExpression::Integer { value, .. }
                            if *value == BigInt::from(index))) => Some(assignment),
            _ => None,
        }).expect("canonical initial constant array cell write");
        assert_eq!(assignment.values.len(), 1);
        let ScalarExpression::FunctionLiteral { span, .. } = &assignment.values[0] else {
            panic!("actual initial callable cell literal")
        };
        let literal = SourceSpan::new(source.clone(), *span);
        assert_ne!(literal, a);
        assert_ne!(literal, b);
        assert!(caller_cfg.environments.iter().any(|environment| environment.literal == literal));
        literal
    });
    assert_ne!(initial_literals[0], initial_literals[1]);
    assert_eq!(inspect.parameters, vec!["items"]);
    assert_eq!(inspect.parameter_spans.len(), 1);
    assert_eq!(callee_cfg.backing_parameters.len(), 1);
    let parameter = &callee_cfg.backing_parameters[0];
    assert_eq!(parameter.source, source);
    assert_eq!(parameter.function_span, inspect.span);
    assert_eq!(parameter.declaration_span, inspect.parameter_spans[0]);
    assert_eq!(parameter.kind, ReferenceBindingKind::Declared);
    assert_eq!(parameter.block_span, inspect.body.span);
    let call_expression = exercise
        .body
        .items
        .iter()
        .find_map(|item| match item {
            ScalarBlockItem::Expression(expression @ ScalarExpression::Call { name, .. })
                if name == "inspect" =>
            {
                Some(expression)
            }
            _ => None,
        })
        .expect("canonical inspect(items) occurrence");
    let ScalarExpression::Call {
        span,
        arguments,
        occurrence,
        ..
    } = call_expression
    else {
        unreachable!()
    };
    assert_eq!(arguments.len(), 1);
    assert!(matches!(&arguments[0], ScalarExpression::Name { name, .. } if name == "items"));
    let call_span = SourceSpan::new(source.clone(), *span);
    assert_eq!(occurrence.source_span, call_span);
    let call_points = caller_cfg.points.iter().filter(|point| {
        matches!(&point.kind, CfgPointKind::Call { expression, .. } if expression == call_expression)
    }).collect::<Vec<_>>();
    assert_eq!(call_points.len(), 1);
    let call = call_points[0];
    assert_eq!(call.source_span, call_span);
    let CfgPointKind::Call {
        target,
        argument_points,
        ..
    } = &call.kind
    else {
        unreachable!()
    };
    assert!(matches!(target, ResolvedCallTarget::ModuleCallable {
        source: target_source, declaration_span, concrete: ConcreteCallSelection::Declaration
    } if target_source == &source && *declaration_span == inspect.name_span));
    assert_eq!(argument_points.len(), 1);
    assert!(
        matches!(&caller_cfg.initial_points[argument_points[0].0].kind,
        CfgPointKind::Read { binding } if binding == local_owner)
    );
    let aliases = [0, 1].map(|index| {
        let name = ["left", "right"][index as usize];
        let pinned_binding = exercise.body.items.iter().find_map(|item| match item {
            ScalarBlockItem::LocalBinding(binding) if binding.name == name => Some(binding),
            _ => None,
        }).expect("canonical common pinned binding");
        let ScalarExpression::CheckedAddress { span: borrow_span, mutability, .. } = &pinned_binding.value else {
            panic!("canonical common mutable borrow operand")
        };
        assert_eq!(*mutability, ScalarReferenceMutability::Mutable);
        let borrow = caller_cfg.initial_points.iter().find_map(|point| match &point.kind {
            CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { place, .. } }
                if point.source_span.range == *borrow_span
                && place.binding == *local_owner && place.projections.iter().any(|projection| {
                    matches!(projection, ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. })
                        if *value == BigInt::from(index))
                }) => Some(point),
            _ => None,
        }).expect("actual common constant cell borrow producer");
        assert_eq!(borrow.source_span.source, source);
        let initialized = caller.observations.points.iter().filter(|snapshot| {
            snapshot.point == borrow.id && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation
        }).collect::<Vec<_>>();
        assert_eq!(initialized.len(), 2);
        if index == 0 {
            for snapshot in initialized {
                let backings = &snapshot.state.backing_bindings[local_owner];
                assert_eq!(backings.len(), 1);
                assert_eq!(backings[0].callable_family.len(), 1);
                for partition in &backings[0].callable_family[0].partitions {
                    for (cell, literal) in initial_literals.iter().enumerate() {
                        let representative = partition.representatives.iter().find(|representative| {
                            representative.key == CallableIndexKeyFact::Constant(cell.into())
                        }).expect("actual initialized constant cell before common writes");
                        assert_eq!(representative.cell.alternatives.len(), 1);
                        assert!(matches!(&representative.cell.alternatives[0],
                            CallableCellCaseFact::Initialized { environment_origin, backing, owner, .. }
                                if &environment_origin.literal == literal
                                && backing == &backings[0].value && owner == local_owner));
                    }
                }
            }
        }
        let CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { loan, .. } } = &borrow.kind else {
            unreachable!()
        };
        assert_eq!(loan.origin_place.binding, *local_owner);
        assert_eq!(loan.creation_source, source);
        let writes = caller_cfg.points.iter().filter_map(|point| match &point.kind {
            CfgPointKind::AssignThrough { candidates, .. }
                if candidates.iter().any(|candidate| candidate.loan == *loan) => Some((point, candidates)),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].1.len(), 1);
        assert!(writes[0].0.id < call.id);
        assert!(matches!(&writes[0].0.kind, CfgPointKind::AssignThrough {
            place: ScalarPlace::Dereference { pointer, .. }, ..
        } if matches!(pointer.as_ref(), ScalarExpression::Name { name: actual, .. } if actual == name)));
        let alias = available_place(&writes[0].1[0].place);
        assert_eq!(alias.binding, *local_owner);
        assert_eq!(alias.source, source);
        assert!(alias.projections.iter().any(|projection| matches!(projection,
            ReferencePlaceProjection::Index(ScalarExpression::Integer { value, .. })
                if *value == BigInt::from(index))));
        alias
    });
    assert_ne!(aliases[0], aliases[1]);
    let caller_snapshots = caller
        .observations
        .points
        .iter()
        .filter(|snapshot| {
            snapshot.point == call.id && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation
        })
        .collect::<Vec<_>>();
    assert_eq!(caller_snapshots.len(), 2);
    let mut caller_relation = Vec::new();
    let mut caller_backings = Vec::new();
    for snapshot in caller_snapshots {
        let backings = &snapshot.state.backing_bindings[local_owner];
        assert_eq!(backings.len(), 1);
        let backing = &backings[0];
        assert_eq!(backing.owner, *local_owner);
        assert_eq!(backing.value.source, source);
        assert_eq!(backing.value.function_span, exercise.span);
        assert_eq!(backing.value.creation_span, items_binding.name_span);
        assert_eq!(backing.value.position, RuntimeValuePosition::Local);
        assert_eq!(backing.origin, creation.source_span);
        assert_eq!(backing.length, length_site.length);
        for tuple in joint_tuples(backing, &aliases) {
            if !caller_relation.contains(&tuple) {
                caller_relation.push(tuple);
            }
        }
        caller_backings.push(backing);
    }
    assert_relation(&caller_relation, &a, &b, diagonal);
    assert_eq!(call.backing_argument_paths.len(), 2);
    let mut argument_relation = Vec::new();
    let mut argument_backings = Vec::new();
    for path in &call.backing_argument_paths {
        assert_eq!(path.len(), 1);
        let argument = &path[0];
        assert_eq!(argument.position, 0);
        assert!(argument.projections.is_empty());
        let backing = &argument.backing;
        assert_eq!(backing.owner, *parameter);
        let tuples = joint_tuples(backing, &aliases);
        assert_eq!(tuples.len(), 1);
        let original = caller_backings
            .iter()
            .find(|original| joint_tuples(original, &aliases) == tuples)
            .expect("one actual caller whole tuple for this argument path");
        assert_eq!(backing.value, original.value);
        assert_eq!(backing.origin, original.origin);
        assert_eq!(backing.length, original.length);
        for tuple in tuples {
            if !argument_relation.contains(&tuple) {
                argument_relation.push(tuple);
            }
        }
        argument_backings.push(backing);
    }
    assert_relation(&argument_relation, &a, &b, diagonal);
    let outgoing_calls = caller
        .observations
        .points
        .iter()
        .filter(|snapshot| {
            snapshot.point == call.id
                && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup
        })
        .collect::<Vec<_>>();
    assert_eq!(outgoing_calls.len(), 2);
    let mut observed_argument_relation = Vec::new();
    for snapshot in outgoing_calls {
        assert!(!snapshot.state.backing_bindings.contains_key(local_owner));
        let paths = snapshot
            .state
            .backing_call_paths
            .iter()
            .find(|(span, _)| span == &call_span)
            .expect("actual same-call whole argument path observation");
        assert_eq!(paths.1.len(), 1);
        assert_eq!(paths.1[0].len(), 1);
        let argument = &paths.1[0][0];
        assert_eq!(argument.position, 0);
        assert!(argument_backings.contains(&&argument.backing));
        for tuple in joint_tuples(&argument.backing, &aliases) {
            if !observed_argument_relation.contains(&tuple) {
                observed_argument_relation.push(tuple);
            }
        }
    }
    assert_relation(&observed_argument_relation, &a, &b, diagonal);
    let input_paths = call
        .backing_argument_paths
        .iter()
        .map(|path| {
            path.iter()
                .map(|argument| argument.backing.clone())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(callee_cfg.backing_input_paths, input_paths);
    let entry_points = callee_cfg
        .initial_points
        .iter()
        .filter(|point| matches!(point.kind, CfgPointKind::Entry))
        .collect::<Vec<_>>();
    assert_eq!(entry_points.len(), 1);
    let entry = entry_points[0];
    assert_eq!(entry.source_span.source, source);
    let entries = callee
        .observations
        .points
        .iter()
        .filter(|snapshot| {
            snapshot.point == entry.id && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation
        })
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 2);
    let mut entry_relation = Vec::new();
    for snapshot in entries {
        let backings = &snapshot.state.backing_bindings[parameter];
        assert_eq!(backings.len(), 1);
        let backing = &backings[0];
        assert_eq!(backing.owner, *parameter);
        assert!(argument_backings.contains(&backing));
        for tuple in joint_tuples(backing, &aliases) {
            if !entry_relation.contains(&tuple) {
                entry_relation.push(tuple);
            }
        }
    }
    assert_relation(&entry_relation, &a, &b, diagonal);
}

#[test]
fn compositional_assignment_result_family_joint_crossed_entry_scalar_program_native64() {
    source_entry_correlation(ScalarTargetLayout::NATIVE64, false, false);
}

#[test]
fn compositional_assignment_result_family_joint_crossed_entry_scalar_program_wasm32() {
    source_entry_correlation(ScalarTargetLayout::WASM32, false, false);
}

#[test]
fn compositional_assignment_result_family_joint_crossed_entry_scalar_project_native64() {
    source_entry_correlation(ScalarTargetLayout::NATIVE64, true, false);
}

#[test]
fn compositional_assignment_result_family_joint_crossed_entry_scalar_project_wasm32() {
    source_entry_correlation(ScalarTargetLayout::WASM32, true, false);
}

#[test]
fn compositional_assignment_result_family_joint_diagonal_entry_scalar_program_native64() {
    source_entry_correlation(ScalarTargetLayout::NATIVE64, false, true);
}

#[test]
fn compositional_assignment_result_family_joint_diagonal_entry_scalar_program_wasm32() {
    source_entry_correlation(ScalarTargetLayout::WASM32, false, true);
}

#[test]
fn compositional_assignment_result_family_joint_diagonal_entry_scalar_project_native64() {
    source_entry_correlation(ScalarTargetLayout::NATIVE64, true, true);
}

#[test]
fn compositional_assignment_result_family_joint_diagonal_entry_scalar_project_wasm32() {
    source_entry_correlation(ScalarTargetLayout::WASM32, true, true);
}

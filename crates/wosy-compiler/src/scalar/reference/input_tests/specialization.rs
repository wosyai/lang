use super::*;

const GENERIC: &str = "%%start\nforward = overload { generic T; T(T) => fn(input) { input }; };\nu8(u8) caller = fn(input) { u8 first = forward(input); forward(first) };\n%%end";

fn raw_module(identity: SourceIdentity, text: &str, target: ScalarTargetLayout) -> ScalarModule {
    let parsed = parse_source(identity, text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let ScalarSourceDeclarations {
        source,
        items,
        structs,
        enums,
        diagnostics,
    } = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut module = ScalarModule::new(source, items, Vec::new());
    module.structs = structs;
    module.enums = enums;
    module.target_layout = target;
    module
}

fn assert_policies(
    caller: &ScalarFunction,
    template: &ScalarFunction,
    source: &SourceIdentity,
    ty: ScalarType,
    policy: CopyPolicy,
) {
    let arm = &template.overload_arms[0];
    let deferred = arm
        .reference_cfg
        .points
        .iter()
        .filter(|point| {
            matches!(
                point.kind,
                CfgPointKind::DeferredMove { .. }
                    | CfgPointKind::DeferredMoveThrough { .. }
                    | CfgPointKind::DeferredArrayElement { .. }
            )
        })
        .collect::<Vec<_>>();
    assert!(!deferred.is_empty());
    let mut expected = Vec::new();
    for call in &caller.reference_cfg.points {
        if let CfgPointKind::Call {
            output: 0,
            target:
                ResolvedCallTarget::ModuleCallable {
                    source: target,
                    declaration_span,
                    concrete: ConcreteCallSelection::Overload(selection),
                },
            ..
        } = &call.kind
        {
            assert_eq!(target, source);
            assert_eq!(*declaration_span, template.name_span);
            assert_eq!(selection.arm_index, 0);
            for point in &deferred {
                expected.push(SpecializedCopyPolicy {
                    call_point: call.id,
                    template_source: source.clone(),
                    template_function_span: arm.span,
                    template_point: point.id,
                    ty: ty.clone(),
                    policy,
                });
            }
        }
    }
    assert!(!expected.is_empty());
    assert_eq!(caller.reference_cfg.specialized_policies, expected);
}

#[test]
fn actual_standalone_specialization_has_only_concrete_template_scratch_copies() {
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), GENERIC.into(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let module = ScalarModule::from_program(program.clone(), Vec::new());
        let sites = resolve_module_call_sites(&program.source, &program.items, &[module]).unwrap();
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let errors = record_program_reference_origins(&mut program, &sites);
        assert!(errors.is_empty(), "{errors:?}");
        let copies = scope.counts();
        assert_eq!(copies.program_copies, 0);
        assert_eq!(copies.module_constructions, 0);
        assert_eq!(copies.module_copies, 0);
        assert_eq!(copies.cfg_copies, 2);
        assert_eq!(stages.counts().copy_specialization, 1);
        drop(stages);
        drop(scope);
        assert_eq!(program, public.program);
        let ScalarItem::Function(template) = &program.items[0] else {
            panic!("template")
        };
        let ScalarItem::Function(caller) = &program.items[1] else {
            panic!("caller")
        };
        assert_policies(
            caller,
            template,
            &program.source,
            ScalarType::U8,
            CopyPolicy::Copy,
        );
    }
}

#[test]
fn actual_project_specialization_reuses_one_inventory_and_retains_imported_arm_policy_order() {
    let main = "%%start\nlibrary = namespace package \"library.w\";\nstruct copy Point { i32 value; }\nstruct Payload { i32 value; }\nPoint(Point) copied = fn(input) { library.forward(input) };\nPayload(Payload) moved = fn(input) { library.forward(input) };\n%%end";
    let library = "%%start\nforward = overload { generic T; T(T) => fn(input) { input }; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let library_source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "library.w".into(),
            "r1".into(),
        );
        let mut main_module = raw_module(source(), main, target);
        main_module.namespace_bindings.push(ScalarNamespaceBinding {
            binding: "library".into(),
            target: library_source.clone(),
            span: ByteSpan::new(0, 0),
        });
        let raw = ScalarProject::new(
            vec![
                main_module,
                raw_module(library_source.clone(), library, target),
            ],
            vec![source(), library_source.clone()],
        );
        let public_input = raw.clone();
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let public = validate_scalar_project(public_input);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
        assert_eq!(scope.counts().program_copies, 0);
        assert_eq!(scope.counts().module_copies, 0);
        assert_eq!(scope.counts().module_constructions, 0);
        assert_eq!(scope.counts().cfg_copies, 2);
        assert_eq!(scope.counts().struct_copies, 4);
        assert_eq!(stages.counts().copy_specialization, 2);
        drop(stages);
        drop(scope);
        let typed = typecheck_scalar_project(raw);
        assert!(typed.diagnostics.is_empty());
        let mut derived = typed.project;
        let scope = layout::observations::Scope::new();
        assert!(record_project_reference_origins(&mut derived.modules).is_empty());
        assert_eq!(scope.counts().struct_copies, 2);
        assert_eq!(scope.counts().module_copies, 0);
        assert_eq!(scope.counts().cfg_copies, 2);
        drop(scope);
        assert_eq!(derived, public.project);
        let ScalarItem::Function(template) = &derived.modules[1].items[0] else {
            panic!("template")
        };
        for (item, structure, policy) in [
            (
                &derived.modules[0].items[1],
                &derived.modules[0].structs[0],
                CopyPolicy::Copy,
            ),
            (
                &derived.modules[0].items[2],
                &derived.modules[0].structs[1],
                CopyPolicy::Move,
            ),
        ] {
            let ScalarItem::Function(caller) = item else {
                panic!("caller")
            };
            assert_policies(
                caller,
                template,
                &library_source,
                ScalarType::Struct(structure.id.clone()),
                policy,
            );
        }
    }
}

#[test]
fn template_index_retains_first_source_even_when_its_module_is_empty() {
    let first = ScalarModule::new(source(), Vec::new(), Vec::new());
    let second = raw_module(source(), GENERIC, ScalarTargetLayout::WASM32);
    let mut first_results = Vec::new();
    let (mut second_results, errors) = derive_module_reference_origins(
        &second.source,
        &second.items,
        &[],
        &[],
        &[],
        second.resolved_assignment_outputs.as_map(),
    );
    assert!(!errors.is_empty());
    let views = [
        ModuleReferenceView {
            source: &first.source,
            items: &first.items,
            results: &mut first_results,
        },
        ModuleReferenceView {
            source: &second.source,
            items: &second.items,
            results: &mut second_results,
        },
    ];
    let index = ReferenceTemplateIndex::new(&views);
    assert_eq!(index.modules.len(), 1);
    assert!(index.modules.get(&source()).unwrap().is_empty());
}

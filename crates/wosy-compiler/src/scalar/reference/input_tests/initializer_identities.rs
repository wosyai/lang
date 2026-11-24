use super::*;

const TEXT: &str = "%%start
u8 first, u8 second = 1, 2;
u8 copied = second;
*?u8 empty = null;
u8[2] bytes = [0, 1];
copied = bytes[0];
if (true) { next = copied; next = second; next; local = bytes; local[0]; };
%%end";

fn token(start: usize, name: &str) -> ByteSpan {
    ByteSpan::new(start as u32, (start + name.len()) as u32)
}

fn assert_identities(calls: &mut ModuleCallSites, source: &SourceIdentity) {
    let ModuleInitializationDetail::Occurrences(topology) = &mut calls.initialization.detail else {
        panic!("initializer topology");
    };
    let occurrence = |span, role| {
        OccurrenceOrdinal(
            topology
                .occurrences
                .iter()
                .position(|metadata| metadata.span == span && metadata.role == role)
                .unwrap(),
        )
    };
    let second_declaration = token(TEXT.find("second").unwrap(), "second");
    let second_use = token(TEXT.find("= second").unwrap() + 2, "second");
    let name = occurrence(second_use, OccurrenceRole::Expression);
    assert_eq!(
        topology.identities.names[&name],
        InitializationName::Storage(InitializationStorage::Module(second_declaration))
    );
    let null = occurrence(
        token(TEXT.find("null").unwrap(), "null"),
        OccurrenceRole::Expression,
    );
    assert_eq!(topology.identities.names[&null], InitializationName::Null);

    let copied_declaration = token(TEXT.find("copied").unwrap(), "copied");
    let copied_use = token(TEXT.find("copied = bytes").unwrap(), "copied");
    let destination = occurrence(copied_use, OccurrenceRole::Place);
    assert_eq!(
        topology.identities.destinations[&destination],
        InitializationDestination::Existing(InitializationStorage::Module(copied_declaration))
    );
    let bytes_declaration = token(TEXT.find("bytes").unwrap(), "bytes");
    let bytes_use = token(TEXT.find("bytes[0]").unwrap(), "bytes");
    let root = occurrence(bytes_use, OccurrenceRole::Place);
    assert_eq!(
        topology.identities.roots[&root],
        InitializationStorage::Module(bytes_declaration)
    );

    let block = topology
        .structure
        .blocks
        .iter()
        .find(|block| block.span.start == TEXT.find("{ next").unwrap() as u32)
        .unwrap();
    let next = token(TEXT.find("next = copied").unwrap(), "next");
    let next_id = InitializationLocal {
        declaration_span: next,
        block_span: block.span,
    };
    let intro = occurrence(next, OccurrenceRole::Place);
    let update = occurrence(
        token(TEXT.find("next = second").unwrap(), "next"),
        OccurrenceRole::Place,
    );
    let read = occurrence(
        token(TEXT.find("next;").unwrap(), "next"),
        OccurrenceRole::Expression,
    );
    assert_eq!(
        topology.identities.destinations[&intro],
        InitializationDestination::Introduce(next_id)
    );
    assert_eq!(
        topology.identities.destinations[&update],
        InitializationDestination::Existing(InitializationStorage::Lexical(next_id))
    );
    assert_eq!(
        topology.identities.names[&read],
        InitializationName::Storage(InitializationStorage::Lexical(next_id))
    );
    let local = token(TEXT.find("local =").unwrap(), "local");
    let local_root = occurrence(
        token(TEXT.find("local[0]").unwrap(), "local"),
        OccurrenceRole::Place,
    );
    assert_eq!(
        topology.identities.roots[&local_root],
        InitializationStorage::Lexical(InitializationLocal {
            declaration_span: local,
            block_span: block.span
        })
    );
    assert!(topology
        .assignments
        .values()
        .any(|group| group.destinations().any(|slot| {
            slot.destination == intro
                && slot.replacement == AssignmentReplacement::Resolved(ScalarType::U8)
        })));

    assert!(name.0 < destination.0 || name.0 < root.0);
    let mut corrupt = calls.clone();
    let ModuleInitializationDetail::Occurrences(topology) = &mut corrupt.initialization.detail
    else {
        unreachable!()
    };
    assert!(topology.identities.roots.remove(&root).is_some());
    assert!(topology
        .identities
        .destinations
        .remove(&destination)
        .is_some());
    assert!(topology.identities.names.remove(&name).is_some());
    let stages = stage_observations::Scope::new();
    assert_eq!(
        occurrence_obligations::validate_module(&corrupt),
        Err(ReferenceAnalysisError::MissingContext {
            source_span: SourceSpan::new(source.clone(), second_use),
            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
        })
    );
    assert_eq!(stages.counts(), stage_observations::Counts::default());
}

#[test]
fn initializer_publishes_ordinal_identities_and_rejects_first_missing_row_on_both_routes_and_layouts(
) {
    let source = source();
    let parsed = parse_source(source.clone(), TEXT.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = || derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    let lookup_only = stage_observations::Counts {
        call_sites: 1,
        ..Default::default()
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let (typed, occurrences) =
            typecheck_scalar_program_with_occurrences(declarations(), target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let stages = stage_observations::Scope::new();
        let mut single =
            resolve_program_call_sites_with_occurrences(&typed.program, occurrences).unwrap();
        assert_eq!(stages.counts(), lookup_only);
        drop(stages);
        assert_identities(&mut single, &source);
        let raw = resolve_program_call_sites(&typed.program).unwrap();
        let ModuleInitializationDetail::Occurrences(raw) = &raw.initialization.detail else {
            unreachable!()
        };
        assert_eq!(raw.identities, InitializationIdentities::default());

        let ScalarSourceDeclarations {
            items,
            structs,
            enums,
            ..
        } = declarations();
        let mut module = ScalarModule::new(source.clone(), items, Vec::new());
        module.structs = structs;
        module.enums = enums;
        module.target_layout = target;
        let (typed, occurrences) = typecheck_scalar_project_with_occurrences(ScalarProject::new(
            vec![module],
            vec![source.clone()],
        ));
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let stages = stage_observations::Scope::new();
        let mut project =
            resolve_project_call_sites_with_occurrences(&typed.project.modules, occurrences);
        assert_eq!(stages.counts(), lookup_only);
        drop(stages);
        assert_identities(project.modules[0].as_mut().unwrap(), &source);
    }
}

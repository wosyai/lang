use super::*;

pub(super) struct ScalarDeclarationEnvironment {
    modules: Vec<ScalarModuleDeclarations>,
    sources: HashMap<SourceIdentity, usize>,
}

struct ScalarModuleDeclarations {
    source: SourceIdentity,
    local_types: BTreeMap<String, ScalarType>,
    namespaces: BTreeMap<String, SourceIdentity>,
    exported_structs: BTreeMap<String, ScalarStructId>,
    exported_enums: BTreeMap<String, ScalarEnumId>,
}

pub(super) fn resolve_project_types(modules: &mut [ScalarModule]) -> ScalarDeclarationEnvironment {
    let declarations = ScalarDeclarationEnvironment::new(modules);
    for (ordinal, module) in modules.iter_mut().enumerate() {
        resolve_module_types_in_project(module, &declarations, ordinal);
    }
    declarations
}

impl ScalarDeclarationEnvironment {
    pub(super) fn new(modules: &[ScalarModule]) -> Self {
        #[cfg(test)]
        observations::record_environment();
        let mut sources = HashMap::new();
        let modules = modules
            .iter()
            .enumerate()
            .map(|(ordinal, module)| {
                let header = ScalarModuleDeclarations::new(module);
                if !sources.contains_key(&header.source) {
                    sources.insert(copy_source(&header.source), ordinal);
                }
                header
            })
            .collect();
        Self { modules, sources }
    }

    pub(super) fn local_types(&self, module: usize) -> &BTreeMap<String, ScalarType> {
        &self.modules[module].local_types
    }

    pub(super) fn qualified_type(
        &self,
        module: usize,
        namespace: &str,
        name: &str,
    ) -> Option<ScalarType> {
        let source = self.modules[module].namespaces.get(namespace)?;
        let target = &self.modules[*self.sources.get(source)?];
        if let Some(id) = target.exported_structs.get(name) {
            return Some(ScalarType::Struct(id.clone()));
        }
        target
            .exported_enums
            .get(name)
            .map(|id| ScalarType::Enum(id.clone()))
    }
}

impl ScalarModuleDeclarations {
    fn new(module: &ScalarModule) -> Self {
        #[cfg(test)]
        observations::record_module();
        let mut local_types = BTreeMap::new();
        let mut exported_structs = BTreeMap::new();
        let mut exported_enums = BTreeMap::new();
        for structure in &module.structs {
            local_types.insert(
                copy_name(&structure.name),
                ScalarType::Struct(copy_struct_id(&structure.id)),
            );
            if !is_module_private_name(&structure.name) {
                exported_structs
                    .entry(copy_name(&structure.name))
                    .or_insert_with(|| copy_struct_id(&structure.id));
            }
        }
        for enumeration in &module.enums {
            local_types.insert(
                copy_name(&enumeration.name),
                ScalarType::Enum(copy_enum_id(&enumeration.id)),
            );
            if !is_module_private_name(&enumeration.name) {
                exported_enums
                    .entry(copy_name(&enumeration.name))
                    .or_insert_with(|| copy_enum_id(&enumeration.id));
            }
        }
        let mut namespaces = BTreeMap::new();
        for namespace in &module.namespace_bindings {
            namespaces
                .entry(copy_name(&namespace.binding))
                .or_insert_with(|| copy_source(&namespace.target));
        }
        Self {
            source: copy_source(&module.source),
            local_types,
            namespaces,
            exported_structs,
            exported_enums,
        }
    }
}

fn copy_name(name: &str) -> String {
    #[cfg(test)]
    observations::record_name(name.len());
    name.to_owned()
}

fn copy_source(source: &SourceIdentity) -> SourceIdentity {
    #[cfg(test)]
    observations::record_source(source);
    source.clone()
}

fn copy_struct_id(id: &ScalarStructId) -> ScalarStructId {
    #[cfg(test)]
    observations::record_struct_id();
    ScalarStructId {
        source: copy_source(&id.source),
        index: id.index,
    }
}

fn copy_enum_id(id: &ScalarEnumId) -> ScalarEnumId {
    #[cfg(test)]
    observations::record_enum_id();
    ScalarEnumId {
        source: copy_source(&id.source),
        index: id.index,
    }
}

#[cfg(test)]
mod observations {
    use super::*;

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub(super) struct Counts {
        pub environments: usize,
        pub modules: usize,
        pub names: usize,
        pub sources: usize,
        pub struct_ids: usize,
        pub enum_ids: usize,
        pub string_bytes: usize,
    }

    thread_local! {
        static ACTIVE: RefCell<Option<Counts>> = const { RefCell::new(None) };
    }

    pub(super) struct Scope {
        previous: Option<Counts>,
    }

    impl Scope {
        pub(super) fn new() -> Self {
            Self {
                previous: ACTIVE.with(|active| active.replace(Some(Counts::default()))),
            }
        }

        pub(super) fn counts(&self) -> Counts {
            ACTIVE.with(|active| {
                active
                    .borrow()
                    .as_ref()
                    .expect("active declaration observations")
                    .clone()
            })
        }
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            ACTIVE.with(|active| active.replace(self.previous.take()));
        }
    }

    fn record(operation: impl FnOnce(&mut Counts)) {
        ACTIVE.with(|active| {
            if let Some(counts) = active.borrow_mut().as_mut() {
                operation(counts);
            }
        });
    }

    pub(super) fn record_environment() {
        record(|counts| counts.environments += 1);
    }
    pub(super) fn record_module() {
        record(|counts| counts.modules += 1);
    }
    pub(super) fn record_name(bytes: usize) {
        record(|counts| {
            counts.names += 1;
            counts.string_bytes += bytes;
        });
    }
    pub(super) fn record_source(source: &SourceIdentity) {
        record(|counts| {
            counts.sources += 1;
            counts.string_bytes += source.project.len()
                + source.package.len()
                + source.path.len()
                + source.revision.len();
        });
    }
    pub(super) fn record_struct_id() {
        record(|counts| counts.struct_ids += 1);
    }
    pub(super) fn record_enum_id() {
        record(|counts| counts.enum_ids += 1);
    }
}

#[cfg(test)]
mod tests {
    use super::super::layout::observations as clone_observations;
    use super::*;
    use crate::parse_source;

    fn source(module: usize) -> SourceIdentity {
        SourceIdentity::new(
            "project".into(),
            "package".into(),
            format!("src/module_{module}.w"),
            "revision".into(),
        )
    }

    fn module(
        source: SourceIdentity,
        text: &str,
        namespaces: Vec<ScalarNamespaceBinding>,
    ) -> ScalarModule {
        let parsed = parse_source(source, text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        ScalarModule::from_program(derive_scalar_program(&parsed.result).program, namespaces)
    }

    fn independent_modules(modules: usize, types: usize, body: usize) -> Vec<ScalarModule> {
        (0..modules).map(|ordinal| {
            let target = source((ordinal + 1) % modules);
            let mut text = format!("%%start\npeer = namespace package \"{}\";\n", target.path);
            for index in 0..types {
                text.push_str(&format!("struct Value{index} {{ *?peer.Value{index} pointer; Kind{index} status; }}\nenum Kind{index} {{ entry; }}\n"));
            }
            text.push_str("unit(peer.Value0) consume = fn(value) { value; };\nu64() payload = fn() { 1");
            for _ in 0..body { text.push_str(" + 1"); }
            text.push_str(" };\n%%end");
            let mut module = module(source(ordinal), &text, Vec::new());
            let ScalarItem::Namespace(namespace) = &module.items[0] else { panic!("peer namespace"); };
            module.namespace_bindings.push(ScalarNamespaceBinding { binding: namespace.binding.clone(), target, span: namespace.span });
            module
        }).collect()
    }

    fn source_bytes(source: &SourceIdentity) -> usize {
        source.project.len() + source.package.len() + source.path.len() + source.revision.len()
    }

    fn enumerate_payload(environment: &ScalarDeclarationEnvironment) -> observations::Counts {
        let mut counts = observations::Counts {
            environments: 1,
            modules: environment.modules.len(),
            ..observations::Counts::default()
        };
        for source in environment.sources.keys() {
            counts.sources += 1;
            counts.string_bytes += source_bytes(source);
        }
        for header in &environment.modules {
            counts.sources += 1;
            counts.string_bytes += source_bytes(&header.source);
            for (name, ty) in &header.local_types {
                counts.names += 1;
                counts.string_bytes += name.len();
                counts.sources += 1;
                counts.string_bytes += match ty {
                    ScalarType::Struct(id) => {
                        counts.struct_ids += 1;
                        source_bytes(&id.source)
                    }
                    ScalarType::Enum(id) => {
                        counts.enum_ids += 1;
                        source_bytes(&id.source)
                    }
                    _ => panic!("declaration-only local type"),
                };
            }
            for (name, source) in &header.namespaces {
                counts.names += 1;
                counts.string_bytes += name.len() + source_bytes(source);
                counts.sources += 1;
            }
            for (name, id) in &header.exported_structs {
                counts.names += 1;
                counts.struct_ids += 1;
                counts.sources += 1;
                counts.string_bytes += name.len() + source_bytes(&id.source);
            }
            for (name, id) in &header.exported_enums {
                counts.names += 1;
                counts.enum_ids += 1;
                counts.sources += 1;
                counts.string_bytes += name.len() + source_bytes(&id.source);
            }
        }
        counts
    }

    fn assert_membership(
        environment: &ScalarDeclarationEnvironment,
        modules: &[ScalarModule],
        types: usize,
    ) {
        assert_eq!(environment.modules.len(), modules.len());
        assert_eq!(environment.sources.len(), modules.len());
        let mut ids = BTreeSet::new();
        for (ordinal, module) in modules.iter().enumerate() {
            let header = &environment.modules[ordinal];
            assert_eq!(header.source, module.source);
            assert_eq!(environment.sources.get(&module.source), Some(&ordinal));
            assert_eq!(header.local_types.len(), types * 2);
            assert_eq!(header.exported_structs.len(), types);
            assert_eq!(header.exported_enums.len(), types);
            assert_eq!(header.namespaces.len(), 1);
            assert_eq!(
                header.namespaces.get("peer"),
                Some(&module.namespace_bindings[0].target)
            );
            let target_ordinal = (ordinal + 1) % modules.len();
            for (index, structure) in module.structs.iter().enumerate() {
                assert!(ids.insert(structure.id.clone()));
                assert_eq!(structure.id.index, index);
                assert_eq!(structure.id.source, module.source);
                assert_eq!(
                    header.local_types.get(&structure.name),
                    Some(&ScalarType::Struct(structure.id.clone()))
                );
                assert_eq!(
                    header.exported_structs.get(&structure.name),
                    Some(&structure.id)
                );
                let target_id = &modules[target_ordinal].structs[index].id;
                assert_eq!(
                    environment.qualified_type(ordinal, "peer", &structure.name),
                    Some(ScalarType::Struct(target_id.clone()))
                );
                assert_eq!(
                    structure.fields[0].ty,
                    ScalarType::RawPointer(Box::new(ScalarType::Struct(target_id.clone())))
                );
                assert_eq!(
                    structure.fields[1].ty,
                    ScalarType::Enum(module.enums[index].id.clone())
                );
            }
            for enumeration in &module.enums {
                assert_eq!(
                    header.local_types.get(&enumeration.name),
                    Some(&ScalarType::Enum(enumeration.id.clone()))
                );
                assert_eq!(
                    header.exported_enums.get(&enumeration.name),
                    Some(&enumeration.id)
                );
            }
        }
        assert_eq!(ids.len(), modules.len() * types);
    }

    #[test]
    fn named_type_context_copies_only_enumerated_headers_across_independent_size_axes() {
        for modules in [4, 8, 16] {
            for types in [4, 8, 16] {
                let mut baseline = None;
                for body in [4, 8, 16] {
                    let mut inputs = independent_modules(modules, types, body);
                    let copy_scope = clone_observations::Scope::new();
                    let header_scope = observations::Scope::new();
                    let environment = resolve_project_types(&mut inputs);
                    assert_membership(&environment, &inputs, types);
                    let counts = header_scope.counts();
                    assert_eq!(counts, enumerate_payload(&environment));
                    assert_eq!(counts.environments, 1);
                    assert_eq!(counts.modules, modules);
                    assert_eq!(counts.names, modules * (4 * types + 1));
                    assert_eq!(counts.sources, modules * (4 * types + 3));
                    assert_eq!(counts.struct_ids, modules * 2 * types);
                    assert_eq!(counts.enum_ids, modules * 2 * types);
                    assert_eq!(copy_scope.counts(), clone_observations::Counts::default());
                    if let Some(previous) = &baseline {
                        assert_eq!(&counts, previous);
                    }
                    baseline = Some(counts);
                }
            }
        }
    }

    #[test]
    fn productive_build_constructs_one_environment_and_keeps_later_clone_obligations_visible() {
        let modules = independent_modules(4, 4, 4);
        let order = modules.iter().map(|module| module.source.clone()).collect();
        let header_scope = observations::Scope::new();
        let copy_scope = clone_observations::Scope::new();
        let validation = validate_scalar_project(ScalarProject::new(modules, order));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        assert_eq!(header_scope.counts().environments, 1);
        assert_eq!(header_scope.counts().modules, 4);
        let copies = copy_scope.counts();
        assert_eq!(copies.module_copies, 0);
        assert!(copies.struct_copies > 0);
        assert_eq!(copies.cfg_copies, 0);
        for (ordinal, module) in validation.project.modules.iter().enumerate() {
            let target = &validation.project.modules[(ordinal + 1) % 4];
            assert_eq!(
                module.structs[0].fields[0].ty,
                ScalarType::RawPointer(Box::new(ScalarType::Struct(target.structs[0].id.clone())))
            );
        }
    }

    #[test]
    fn duplicate_names_namespaces_and_sources_preserve_existing_selection_precedence() {
        let shared_source = source(0);
        let alternative_source = source(1);
        let target = module(shared_source.clone(), "%%start\nstruct Shared { u8 first; }\nstruct Shared { u16 second; }\nenum Shared { one; }\nenum Shared { two; }\nenum OnlyEnum { one; }\nenum OnlyEnum { two; }\nstruct _Hidden { u8 value; }\nenum _HiddenEnum { entry; }\n%%end", Vec::new());
        let duplicate_source = module(
            shared_source.clone(),
            "%%start\nstruct LaterOnly { u8 value; }\nstruct Shared { u32 value; }\n%%end",
            Vec::new(),
        );
        let alternative = module(
            alternative_source.clone(),
            "%%start\nstruct Shared { u64 value; }\n%%end",
            Vec::new(),
        );
        let requester = module(source(2), "%%start\nstruct Request { target.Shared value; target.OnlyEnum status; target._Hidden hidden; target._HiddenEnum hidden_enum; target.LaterOnly absent; other.Shared other; missing.Shared missing; }\n%%end", vec![
            ScalarNamespaceBinding { binding: "target".into(), target: shared_source.clone(), span: ByteSpan::new(1, 2) },
            ScalarNamespaceBinding { binding: "target".into(), target: alternative_source.clone(), span: ByteSpan::new(3, 4) },
            ScalarNamespaceBinding { binding: "other".into(), target: alternative_source, span: ByteSpan::new(5, 6) },
        ]);
        let unresolved = requester.structs[0].fields[2..5]
            .iter()
            .map(|field| field.ty.clone())
            .collect::<Vec<_>>();
        let missing = requester.structs[0].fields[6].ty.clone();
        let mut modules = vec![target, duplicate_source, alternative, requester];
        let environment = resolve_project_types(&mut modules);
        assert_eq!(environment.sources.get(&shared_source), Some(&0));
        assert_eq!(
            environment.local_types(0).get("Shared"),
            Some(&ScalarType::Enum(modules[0].enums[1].id.clone()))
        );
        assert_eq!(
            environment.local_types(0).get("OnlyEnum"),
            Some(&ScalarType::Enum(modules[0].enums[3].id.clone()))
        );
        assert_eq!(
            environment.local_types(1).get("Shared"),
            Some(&ScalarType::Struct(modules[1].structs[1].id.clone()))
        );
        let fields = &modules[3].structs[0].fields;
        assert_eq!(
            fields[0].ty,
            ScalarType::Struct(modules[0].structs[0].id.clone())
        );
        assert_eq!(
            fields[1].ty,
            ScalarType::Enum(modules[0].enums[2].id.clone())
        );
        for (field, expected) in fields[2..5].iter().zip(unresolved) {
            assert_eq!(field.ty, expected);
        }
        assert_eq!(
            fields[5].ty,
            ScalarType::Struct(modules[2].structs[0].id.clone())
        );
        assert_eq!(fields[6].ty, missing);
        assert!(environment.modules[0].local_types.contains_key("_Hidden"));
        assert!(environment.modules[0]
            .local_types
            .contains_key("_HiddenEnum"));
        assert!(!environment.modules[0]
            .exported_structs
            .contains_key("_Hidden"));
        assert!(!environment.modules[0]
            .exported_enums
            .contains_key("_HiddenEnum"));
    }

    #[test]
    fn complete_source_identity_and_first_missing_namespace_target_are_authoritative() {
        let base = source(0);
        let sources = vec![
            base.clone(),
            SourceIdentity {
                project: "other_project".into(),
                ..base.clone()
            },
            SourceIdentity {
                package: "other_package".into(),
                ..base.clone()
            },
            SourceIdentity {
                revision: "other_revision".into(),
                ..base.clone()
            },
        ];
        let mut modules = sources
            .iter()
            .map(|source| {
                module(
                    source.clone(),
                    "%%start\nstruct Item { u8 value; }\n%%end",
                    Vec::new(),
                )
            })
            .collect::<Vec<_>>();
        let mut namespaces = sources
            .iter()
            .enumerate()
            .map(|(index, source)| ScalarNamespaceBinding {
                binding: format!("target{index}"),
                target: source.clone(),
                span: ByteSpan::new(1, 2),
            })
            .collect::<Vec<_>>();
        namespaces.push(ScalarNamespaceBinding {
            binding: "shadow".into(),
            target: source(99),
            span: ByteSpan::new(3, 4),
        });
        namespaces.push(ScalarNamespaceBinding {
            binding: "shadow".into(),
            target: base,
            span: ByteSpan::new(5, 6),
        });
        let requester = module(source(5), "%%start\nstruct Request { target0.Item zero; target1.Item one; target2.Item two; target3.Item three; shadow.Item absent; }\n%%end", namespaces);
        let missing = requester.structs[0].fields[4].ty.clone();
        modules.push(requester);
        let environment = resolve_project_types(&mut modules);
        assert_eq!(environment.sources.len(), 5);
        for (ordinal, source) in sources.iter().enumerate() {
            assert_eq!(environment.sources.get(source), Some(&ordinal));
            let id = &modules[ordinal].structs[0].id;
            assert_eq!(id.index, 0);
            assert_eq!(&id.source, source);
            assert_eq!(
                modules[4].structs[0].fields[ordinal].ty,
                ScalarType::Struct(id.clone())
            );
        }
        assert_eq!(modules[4].structs[0].fields[4].ty, missing);
    }

    #[test]
    fn empty_context_and_unresolved_type_spans_remain_exact() {
        let copy_scope = clone_observations::Scope::new();
        let header_scope = observations::Scope::new();
        let empty = resolve_project_types(&mut []);
        assert!(empty.modules.is_empty());
        assert!(empty.sources.is_empty());
        assert_eq!(
            header_scope.counts(),
            observations::Counts {
                environments: 1,
                ..observations::Counts::default()
            }
        );
        assert_eq!(copy_scope.counts(), clone_observations::Counts::default());
        drop(header_scope);
        drop(copy_scope);
        let unresolved_source = source(0);
        let requester = module(unresolved_source.clone(), "%%start\nstruct Request { Unknown named; absent.Type absent; bound.Type bound; }\n%%end", vec![ScalarNamespaceBinding { binding: "bound".into(), target: source(99), span: ByteSpan::new(1, 2) }]);
        let fields = requester.structs[0]
            .fields
            .iter()
            .map(|field| (field.id.clone(), field.ty.clone(), field.name_span))
            .collect::<Vec<_>>();
        let mut modules = vec![requester];
        let environment = resolve_project_types(&mut modules);
        assert_eq!(environment.modules[0].source, unresolved_source);
        for (field, (id, ty, span)) in modules[0].structs[0].fields.iter().zip(fields) {
            assert_eq!(field.id, id);
            assert_eq!(field.ty, ty);
            assert_eq!(field.name_span, span);
        }
    }
}

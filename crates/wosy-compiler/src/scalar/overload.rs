use super::*;

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ScalarOverloadArmHeader {
    pub signature: ScalarType,
    pub generic_parameters: Vec<ScalarGenericParameter>,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ScalarOverloadHeader {
    pub arms: Vec<ScalarOverloadArmHeader>,
    pub(super) origin: (usize, usize),
}

impl ScalarOverloadHeader {
    fn from_function(function: &ScalarFunction) -> Self {
        #[cfg(test)]
        observations::record_header(function);
        Self {
            origin: (0, 0),
            arms: function
                .overload_arms
                .iter()
                .map(|arm| ScalarOverloadArmHeader {
                    signature: copy_type(&arm.signature),
                    generic_parameters: arm
                        .generic_parameters
                        .iter()
                        .map(|parameter| ScalarGenericParameter {
                            name: copy_name(&parameter.name),
                            span: parameter.span,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

struct ScalarOverloadDeclaration {
    name: String,
    header: ScalarOverloadHeader,
}

struct ScalarOverloadNamespace {
    binding: String,
    target: SourceIdentity,
}

struct ScalarOverloadModule {
    source: SourceIdentity,
    scope: BTreeMap<String, ScalarType>,
    declarations: BTreeMap<usize, ScalarOverloadDeclaration>,
    first_overloads: BTreeMap<String, usize>,
    namespaces: Vec<ScalarOverloadNamespace>,
    callables: ScalarCallableHeaders,
}

struct ScalarCallableHeaders {
    generics: BTreeMap<String, Vec<ScalarGenericParameter>>,
    externs: BTreeMap<String, BTreeMap<String, ScalarExternMemberHeader>>,
}

pub(super) struct ScalarExternMemberHeader {
    pub signature: ScalarType,
    pub first_declaration: bool,
    pub unsafe_marker: bool,
    pub any_unsafe: bool,
}

impl ScalarCallableHeaders {
    fn new(items: &[ScalarItem]) -> Self {
        let mut generics = BTreeMap::new();
        let mut externs = BTreeMap::<String, BTreeMap<String, ScalarExternMemberHeader>>::new();
        for item in items {
            match item {
                ScalarItem::Function(function) => {
                    if !generics.contains_key(&function.name) {
                        generics.insert(
                            copy_name(&function.name),
                            function
                                .generic_parameters
                                .iter()
                                .map(|parameter| ScalarGenericParameter {
                                    name: copy_name(&parameter.name),
                                    span: parameter.span,
                                })
                                .collect(),
                        );
                    }
                }
                ScalarItem::Extern(declaration) => {
                    let first_declaration = !externs.contains_key(&declaration.binding);
                    if first_declaration {
                        externs.insert(copy_name(&declaration.binding), BTreeMap::new());
                    }
                    let members = externs
                        .get_mut(&declaration.binding)
                        .expect("indexed extern binding");
                    for function in &declaration.functions {
                        if let Some(member) = members.get_mut(&function.name) {
                            member.any_unsafe |= function.unsafe_marker;
                        } else {
                            members.insert(
                                copy_name(&function.name),
                                ScalarExternMemberHeader {
                                    signature: copy_type(&function.signature),
                                    first_declaration,
                                    unsafe_marker: function.unsafe_marker,
                                    any_unsafe: function.unsafe_marker,
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        let headers = Self { generics, externs };
        #[cfg(test)]
        observations::record_callables(&headers);
        headers
    }
}

#[derive(Clone, Copy)]
pub(super) struct ScalarTypingView<'a> {
    environment: &'a ScalarOverloadEnvironment,
    ordinal: usize,
}

impl ScalarTypingView<'_> {
    pub(super) fn module_ordinal_for_source(&self, source: &SourceIdentity) -> Option<usize> {
        self.environment.sources.get(source).copied()
    }

    pub(super) fn imported(&self, source: &SourceIdentity) -> Self {
        Self {
            environment: self.environment,
            ordinal: *self
                .environment
                .sources
                .get(source)
                .expect("typing source in phase environment"),
        }
    }
    pub(super) fn generics(&self, name: &str) -> Option<&[ScalarGenericParameter]> {
        self.environment.modules[self.ordinal]
            .callables
            .generics
            .get(name)
            .map(Vec::as_slice)
    }
    pub(super) fn extern_member(
        &self,
        binding: Option<&str>,
        name: &str,
    ) -> Option<&ScalarExternMemberHeader> {
        self.environment.modules[self.ordinal]
            .callables
            .externs
            .get(binding?)?
            .get(name)
    }
    pub(super) fn first(
        &self,
        source: &SourceIdentity,
        name: &str,
    ) -> Option<&ScalarOverloadHeader> {
        self.environment.first(source, name)
    }
    pub(super) fn for_call(
        &self,
        source: &SourceIdentity,
        receiver: Option<&str>,
        name: &str,
    ) -> Option<&ScalarOverloadHeader> {
        self.environment.for_call(source, receiver, name)
    }
}

pub(super) struct ScalarOverloadEnvironment {
    modules: Vec<ScalarOverloadModule>,
    sources: HashMap<SourceIdentity, usize>,
}

impl ScalarOverloadEnvironment {
    pub(super) fn typing(&self, ordinal: usize) -> ScalarTypingView<'_> {
        ScalarTypingView {
            environment: self,
            ordinal,
        }
    }
    pub(super) fn single(source: &SourceIdentity, items: &[ScalarItem]) -> Self {
        #[cfg(test)]
        let _snapshot = observations::SnapshotScope::new();
        #[cfg(test)]
        observations::record_context(1);
        let scope = items
            .iter()
            .filter_map(|item| match item {
                ScalarItem::Binding(binding) => {
                    Some((copy_name(&binding.name), copy_type(&binding.declared_type)))
                }
                ScalarItem::Function(function) => {
                    Some((copy_name(&function.name), copy_type(&function.signature)))
                }
                ScalarItem::Namespace(_) | ScalarItem::Extern(_) | ScalarItem::Executable(_) => {
                    None
                }
            })
            .collect();
        let declarations = overload_declarations(items, 0);
        let module = ScalarOverloadModule {
            callables: ScalarCallableHeaders::new(items),
            source: copy_source(source),
            scope,
            first_overloads: first_overloads(&declarations),
            declarations,
            namespaces: Vec::new(),
        };
        Self {
            sources: HashMap::from([(copy_source(&module.source), 0)]),
            modules: vec![module],
        }
    }

    pub(super) fn project(modules: &[ScalarModule]) -> Self {
        #[cfg(test)]
        let _snapshot = observations::SnapshotScope::new();
        #[cfg(test)]
        observations::record_context(modules.len());
        let mut sources = HashMap::new();
        let modules = modules
            .iter()
            .enumerate()
            .map(|(ordinal, module)| {
                let declarations = overload_declarations(&module.items, ordinal);
                let header = ScalarOverloadModule {
                    callables: ScalarCallableHeaders::new(&module.items),
                    source: copy_source(&module.source),
                    scope: module
                        .members
                        .iter()
                        .map(|(name, ty)| (copy_name(name), copy_type(ty)))
                        .collect(),
                    first_overloads: first_overloads(&declarations),
                    declarations,
                    namespaces: module
                        .namespace_bindings
                        .iter()
                        .map(|namespace| ScalarOverloadNamespace {
                            binding: copy_name(&namespace.binding),
                            target: copy_source(&namespace.target),
                        })
                        .collect(),
                };
                if !sources.contains_key(&header.source) {
                    sources.insert(copy_source(&header.source), ordinal);
                }
                header
            })
            .collect();
        Self { modules, sources }
    }

    fn lookup(&self, ordinal: usize) -> BTreeMap<String, &ScalarOverloadHeader> {
        let module = &self.modules[ordinal];
        let mut overloads = module
            .declarations
            .values()
            .map(|declaration| (lookup_name(&declaration.name), &declaration.header))
            .collect::<BTreeMap<_, _>>();
        for namespace in &module.namespaces {
            if let Some(target) = self
                .sources
                .get(&namespace.target)
                .map(|ordinal| &self.modules[*ordinal])
            {
                for declaration in target.declarations.values() {
                    if target.scope.contains_key(&declaration.name) {
                        let name = format!("{}.{}", namespace.binding, declaration.name);
                        #[cfg(test)]
                        observations::record_lookup_key(name.len());
                        overloads.insert(name, &declaration.header);
                    }
                }
            }
        }
        #[cfg(test)]
        observations::record_lookup(overloads.len());
        overloads
    }
}

impl ScalarOverloadEnvironment {
    pub(super) fn first(
        &self,
        source: &SourceIdentity,
        name: &str,
    ) -> Option<&ScalarOverloadHeader> {
        let module = &self.modules[*self.sources.get(source)?];
        let position = module.first_overloads.get(name)?;
        Some(
            &module
                .declarations
                .get(position)
                .expect("indexed overload header")
                .header,
        )
    }

    pub(super) fn for_call(
        &self,
        source: &SourceIdentity,
        receiver: Option<&str>,
        name: &str,
    ) -> Option<&ScalarOverloadHeader> {
        let module = &self.modules[*self.sources.get(source)?];
        match receiver {
            None => self.first(source, name),
            Some(binding) => {
                let namespace = module
                    .namespaces
                    .iter()
                    .find(|namespace| namespace.binding == binding)?;
                let target = &self.modules[*self.sources.get(&namespace.target)?];
                if target.scope.contains_key(name) {
                    self.first(&target.source, name)
                } else {
                    None
                }
            }
        }
    }
}

fn first_overloads(
    declarations: &BTreeMap<usize, ScalarOverloadDeclaration>,
) -> BTreeMap<String, usize> {
    let mut first = BTreeMap::new();
    for (position, declaration) in declarations {
        first
            .entry(copy_name(&declaration.name))
            .or_insert(*position);
    }
    first
}

fn overload_declarations(
    items: &[ScalarItem],
    module: usize,
) -> BTreeMap<usize, ScalarOverloadDeclaration> {
    items
        .iter()
        .enumerate()
        .filter_map(|(position, item)| match item {
            ScalarItem::Function(function) if !function.overload_arms.is_empty() => Some((
                position,
                ScalarOverloadDeclaration {
                    name: copy_name(&function.name),
                    header: ScalarOverloadHeader {
                        origin: (module, position),
                        ..ScalarOverloadHeader::from_function(function)
                    },
                },
            )),
            _ => None,
        })
        .collect()
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

fn copy_type(ty: &ScalarType) -> ScalarType {
    #[cfg(test)]
    observations::record_type(ty);
    ty.clone()
}

fn lookup_name(name: &str) -> String {
    #[cfg(test)]
    observations::record_lookup_key(name.len());
    name.to_owned()
}

pub(super) fn record_overload_selections(
    program: &mut ScalarProgram,
    environment: &ScalarOverloadEnvironment,
) {
    #[cfg(test)]
    let _recording = observations::RecordingScope::new();
    let scope = &environment.modules[0].scope;
    let overloads = environment.lookup(0);
    for item in &mut program.items {
        match item {
            ScalarItem::Binding(binding) => {
                let expected = binding
                    .receivers
                    .first()
                    .map(|receiver| &receiver.ty)
                    .unwrap_or(&binding.declared_type);
                record_expression_overload_selection(
                    &mut binding.value,
                    Some(expected),
                    &scope,
                    &overloads,
                );
            }
            ScalarItem::Function(function) => {
                let mut body_scope = scope.clone();
                if let ScalarType::Callable { parameters, .. } = &function.signature {
                    for (name, ty) in function.parameters.iter().zip(parameters) {
                        body_scope.insert(name.clone(), ty.clone());
                    }
                }
                record_block_overload_selections(&mut function.body, &body_scope, &overloads);
            }
            ScalarItem::Executable(item) => {
                let mut item_scope = scope.clone();
                record_block_item_overload_selections(item, &mut item_scope, &overloads);
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
        }
    }
}

fn record_expression_overload_selection(
    expression: &mut ScalarExpression,
    expected: Option<&ScalarType>,
    scope: &BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, &ScalarOverloadHeader>,
) {
    match expression {
        ScalarExpression::AnonymousCallable { .. } | ScalarExpression::GeneratedArray { .. } => {}
        ScalarExpression::IndexedInvoke { selected, arguments, .. } => {
            record_expression_overload_selection(selected, None, scope, overloads);
            for argument in arguments {
                record_expression_overload_selection(argument, None, scope, overloads);
            }
        }
        ScalarExpression::Call {
            receiver,
            name,
            arguments,
            overload_selection,
            ..
        } => {
            for argument in arguments.iter_mut() {
                record_expression_overload_selection(argument, None, scope, overloads);
            }
            if let (Some(overload), Some(argument_types)) = (
                overloads.get(
                    &receiver
                        .as_ref()
                        .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}")),
                ),
                arguments
                    .iter()
                    .map(|argument| overload_argument_type(argument, scope))
                    .collect::<Option<Vec<_>>>(),
            ) {
                if let Ok((_, selection)) =
                    resolve_overload_candidate(overload, &argument_types, expected)
                {
                    *overload_selection = Some(selection);
                    #[cfg(test)]
                    observations::record_selection(
                        overload_selection.as_ref().expect("recorded overload"),
                    );
                }
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            record_expression_overload_selection(left, None, scope, overloads);
            record_expression_overload_selection(right, None, scope, overloads);
        }
        ScalarExpression::Unary { operand, .. } => {
            record_expression_overload_selection(operand, expected, scope, overloads)
        }
        ScalarExpression::ValueField { receiver, .. } => {
            record_expression_overload_selection(receiver, None, scope, overloads)
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            record_expression_overload_selection(
                condition,
                Some(&ScalarType::Bool),
                scope,
                overloads,
            );
            record_block_overload_selections(then_branch, scope, overloads);
            record_block_overload_selections(else_branch, scope, overloads);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            record_expression_overload_selection(
                condition,
                Some(&ScalarType::Bool),
                scope,
                overloads,
            );
            record_block_overload_selections(then_branch, scope, overloads);
        }
        ScalarExpression::Block(block) => record_block_overload_selections(block, scope, overloads),
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => match place {
            ScalarPlace::Dereference { pointer, .. } => {
                record_expression_overload_selection(pointer, None, scope, overloads)
            }
            ScalarPlace::Field { base, .. } => {
                if let ScalarPlace::Dereference { pointer, .. } = base.as_mut() {
                    record_expression_overload_selection(pointer, None, scope, overloads);
                }
            }
            ScalarPlace::Index { base, index, .. } => {
                record_place_overload_selections(base, scope, overloads);
                record_expression_overload_selection(
                    index,
                    Some(&ScalarType::U64),
                    scope,
                    overloads,
                );
            }
            ScalarPlace::Name { .. } => {}
        },
        ScalarExpression::IndexedRead { place, .. } => {
            record_place_overload_selections(place, scope, overloads)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                record_expression_overload_selection(&mut field.value, None, scope, overloads);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                record_expression_overload_selection(element, None, scope, overloads);
            }
        }
        ScalarExpression::Name { .. }
        | ScalarExpression::Member { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. } => {}
    }
}

fn record_place_overload_selections(
    place: &mut ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, &ScalarOverloadHeader>,
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { pointer, .. } => {
            record_expression_overload_selection(pointer, None, scope, overloads)
        }
        ScalarPlace::Field { base, .. } => record_place_overload_selections(base, scope, overloads),
        ScalarPlace::Index { base, index, .. } => {
            record_place_overload_selections(base, scope, overloads);
            record_expression_overload_selection(index, Some(&ScalarType::U64), scope, overloads);
        }
    }
}

fn record_block_overload_selections(
    block: &mut ScalarBlock,
    scope: &BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, &ScalarOverloadHeader>,
) {
    let mut scope = scope.clone();
    for item in &mut block.items {
        record_block_item_overload_selections(item, &mut scope, overloads);
    }
    for output in &mut block.final_output_values {
        record_expression_overload_selection(&mut output.value, None, &scope, overloads);
    }
}

fn record_block_item_overload_selections(
    item: &mut ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    overloads: &BTreeMap<String, &ScalarOverloadHeader>,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            let expected = binding
                .receivers
                .first()
                .map(|receiver| &receiver.ty)
                .unwrap_or(&binding.declared_type);
            record_expression_overload_selection(
                &mut binding.value,
                Some(expected),
                &scope,
                overloads,
            );
            scope.insert(binding.name.clone(), binding.declared_type.clone());
            for receiver in &binding.receivers {
                scope.insert(receiver.name.clone(), receiver.ty.clone());
            }
        }
        ScalarBlockItem::Expression(expression) => {
            record_expression_overload_selection(expression, None, &scope, overloads)
        }
        ScalarBlockItem::Assignment(assignment) => {
            record_expression_overload_selection(
                &mut assignment.value,
                scope.get(&assignment.target),
                &scope,
                overloads,
            );
            for (value, target) in assignment.values.iter_mut().zip(&assignment.targets) {
                record_expression_overload_selection(
                    value,
                    scope.get(&target.target),
                    &scope,
                    overloads,
                );
            }
        }
    }
}

pub(super) fn record_project_overload_selections(
    project: &mut ScalarProject,
    environment: &ScalarOverloadEnvironment,
) {
    #[cfg(test)]
    let _recording = observations::RecordingScope::new();
    for (ordinal, module) in project.modules.iter_mut().enumerate() {
        let scope = &environment.modules[ordinal].scope;
        let overloads = environment.lookup(ordinal);
        for item in &mut module.items {
            match item {
                ScalarItem::Binding(binding) => {
                    let expected = binding
                        .receivers
                        .first()
                        .map(|receiver| &receiver.ty)
                        .unwrap_or(&binding.declared_type);
                    record_expression_overload_selection(
                        &mut binding.value,
                        Some(expected),
                        &scope,
                        &overloads,
                    );
                }
                ScalarItem::Function(function) => {
                    let mut body_scope = scope.clone();
                    if let ScalarType::Callable { parameters, .. } = &function.signature {
                        for (name, ty) in function.parameters.iter().zip(parameters) {
                            body_scope.insert(name.clone(), ty.clone());
                        }
                    }
                    record_block_overload_selections(&mut function.body, &body_scope, &overloads);
                }
                ScalarItem::Executable(item) => {
                    let mut item_scope = scope.clone();
                    record_block_item_overload_selections(item, &mut item_scope, &overloads);
                }
                ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
            }
        }
    }
}

#[cfg(test)]
pub(super) mod observations {
    use super::super::layout::observations as clone_observations;
    use super::*;

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub(in crate::scalar) struct Payload {
        pub callable_headers: usize,
        pub ordinary_parameters: usize,
        pub extern_bindings: usize,
        pub extern_members: usize,
        pub headers: usize,
        pub arms: usize,
        pub generics: usize,
        pub type_nodes: usize,
        pub names: usize,
        pub sources: usize,
        pub string_bytes: usize,
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub(in crate::scalar) struct Counts {
        pub contexts: usize,
        pub modules: usize,
        pub recorders: usize,
        pub snapshot: Payload,
        pub projections: Payload,
        pub lookups: usize,
        pub lookup_entries: usize,
        pub lookup_key_bytes: usize,
        pub selection_writes: usize,
        pub substitution_slots: usize,
        pub candidate_calls: usize,
        pub program_copies: usize,
        pub module_copies: usize,
        pub struct_copies: usize,
        pub cfg_copies: usize,
        pub copied_context_payload_units: usize,
        pub construction_program_copies: usize,
        pub construction_module_copies: usize,
        pub construction_struct_copies: usize,
        pub construction_cfg_copies: usize,
        pub construction_payload_units: usize,
    }

    struct State {
        counts: Counts,
        recording: bool,
    }
    thread_local! {
        static ACTIVE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    pub(in crate::scalar) struct Scope {
        previous: Option<State>,
    }
    impl Scope {
        pub(in crate::scalar) fn new() -> Self {
            Self {
                previous: ACTIVE.with(|active| {
                    active.replace(Some(State {
                        counts: Counts::default(),
                        recording: false,
                    }))
                }),
            }
        }
        pub(in crate::scalar) fn counts(&self) -> Counts {
            ACTIVE.with(|active| {
                active
                    .borrow()
                    .as_ref()
                    .expect("active overload observations")
                    .counts
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
            if let Some(state) = active.borrow_mut().as_mut() {
                operation(&mut state.counts);
            }
        });
    }

    fn record_payload(operation: impl FnOnce(&mut Payload)) {
        ACTIVE.with(|active| {
            if let Some(state) = active.borrow_mut().as_mut() {
                operation(if state.recording {
                    &mut state.counts.snapshot
                } else {
                    &mut state.counts.projections
                });
            }
        });
    }

    pub(super) struct RecordingScope {
        clones: Option<clone_observations::Scope>,
        snapshot: bool,
    }
    impl RecordingScope {
        pub(super) fn new() -> Self {
            Self::start(false)
        }

        fn start(snapshot: bool) -> Self {
            let active = ACTIVE.with(|active| {
                if let Some(state) = active.borrow_mut().as_mut() {
                    assert!(!state.recording);
                    state.recording = snapshot;
                    if !snapshot {
                        state.counts.recorders += 1;
                    }
                    true
                } else {
                    false
                }
            });
            Self {
                clones: active.then(clone_observations::Scope::new),
                snapshot,
            }
        }
    }
    impl Drop for RecordingScope {
        fn drop(&mut self) {
            if let Some(scope) = &self.clones {
                let copies = scope.counts();
                record(|counts| {
                    if self.snapshot {
                        counts.construction_program_copies += copies.program_copies;
                        counts.construction_module_copies += copies.module_copies;
                        counts.construction_struct_copies += copies.struct_copies;
                        counts.construction_cfg_copies += copies.cfg_copies;
                        counts.construction_payload_units += copies.copied_payload_units;
                        return;
                    }
                    counts.program_copies += copies.program_copies;
                    counts.module_copies += copies.module_copies;
                    counts.struct_copies += copies.struct_copies;
                    counts.cfg_copies += copies.cfg_copies;
                    counts.copied_context_payload_units += copies.copied_payload_units;
                });
                ACTIVE.with(|active| {
                    active
                        .borrow_mut()
                        .as_mut()
                        .expect("active recording observations")
                        .recording = false
                });
            }
        }
    }

    pub(super) struct SnapshotScope {
        _recording: RecordingScope,
    }
    impl SnapshotScope {
        pub(super) fn new() -> Self {
            Self {
                _recording: RecordingScope::start(true),
            }
        }
    }

    pub(super) fn record_context(modules: usize) {
        record(|counts| {
            counts.contexts += 1;
            counts.modules += modules;
        });
    }
    pub(in crate::scalar) fn record_candidate() {
        record(|counts| counts.candidate_calls += 1);
    }
    pub(super) fn record_header(function: &ScalarFunction) {
        record_payload(|payload| {
            payload.headers += 1;
            payload.arms += function.overload_arms.len();
            payload.generics += function
                .overload_arms
                .iter()
                .map(|arm| arm.generic_parameters.len())
                .sum::<usize>();
        });
    }
    pub(super) fn record_callables(headers: &ScalarCallableHeaders) {
        record_payload(|payload| {
            payload.callable_headers += headers.generics.len();
            payload.ordinary_parameters += headers.generics.values().map(Vec::len).sum::<usize>();
            payload.extern_bindings += headers.externs.len();
            payload.extern_members += headers.externs.values().map(BTreeMap::len).sum::<usize>();
        });
    }
    pub(super) fn record_name(bytes: usize) {
        record_payload(|payload| {
            payload.names += 1;
            payload.string_bytes += bytes;
        });
    }
    pub(super) fn record_source(source: &SourceIdentity) {
        record_payload(|payload| {
            payload.sources += 1;
            payload.string_bytes += source.project.len()
                + source.package.len()
                + source.path.len()
                + source.revision.len();
        });
    }
    pub(super) fn record_type(ty: &ScalarType) {
        record_payload(|payload| payload.type_nodes += 1);
        match ty {
            ScalarType::RawPointer(inner)
            | ScalarType::CheckedReference { inner, .. }
            | ScalarType::Array { element: inner, .. }
            | ScalarType::RuntimeArray { element: inner, .. } => record_type(inner),
            ScalarType::Callable {
                outputs,
                parameters,
            } => {
                for output in &outputs.outputs {
                    record_type(&output.ty);
                }
                for parameter in parameters {
                    record_type(parameter);
                }
            }
            ScalarType::Named { name, .. } => record_name(name.len()),
            ScalarType::Qualified {
                receiver, member, ..
            } => {
                record_name(receiver.len());
                record_name(member.len());
            }
            ScalarType::Struct(id) => record_source(&id.source),
            ScalarType::Enum(id) => record_source(&id.source),
            _ => {}
        }
    }
    pub(super) fn record_lookup(entries: usize) {
        record(|counts| {
            counts.lookups += 1;
            counts.lookup_entries += entries;
        });
    }
    pub(super) fn record_lookup_key(bytes: usize) {
        record(|counts| counts.lookup_key_bytes += bytes);
    }
    pub(super) fn record_selection(selection: &ScalarOverloadSelection) {
        record(|counts| {
            counts.selection_writes += 1;
            counts.substitution_slots += selection.substitutions.len();
        });
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
            format!("src/overloads_{module}.w"),
            "revision".into(),
        )
    }

    fn text(arms: usize, body: usize, sites: usize, peer: Option<&SourceIdentity>) -> String {
        let mut text = String::from("%%start\n");
        if let Some(peer) = peer {
            text.push_str(&format!("peer = namespace package \"{}\";\n", peer.path));
        }
        text.push_str("choose = overload {\n");
        for arm in 1..arms {
            text.push_str(&format!("u64(u8[{arm}]) => fn(value) {{ value; 1"));
            for _ in 0..body {
                text.push_str(" + 1");
            }
            text.push_str(" };\n");
        }
        text.push_str("generic T;\n*?T(*?T) => fn(value) { ");
        for _ in 0..body {
            text.push_str("value; ");
        }
        text.push_str("value };\n};\nu8[1] input = [1];\n*?u8 pointer = null;\n");
        for site in 0..sites {
            let receiver = if peer.is_some() { "peer." } else { "" };
            text.push_str(&format!("u64 result{site} = {receiver}choose(input);\n*?u8 copied{site} = {receiver}choose(pointer);\n"));
        }
        text.push_str("%%end");
        text
    }

    fn program(
        arms: usize,
        body: usize,
        sites: usize,
        ordinal: usize,
        peer: Option<&SourceIdentity>,
    ) -> ScalarProgram {
        let parsed = parse_source(source(ordinal), text(arms, body, sites, peer), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let validation = derive_scalar_program(&parsed.result);
        if peer.is_none() {
            assert!(
                validation.diagnostics.is_empty(),
                "{:?}",
                validation.diagnostics
            );
        }
        validation.program
    }

    fn project(modules: usize, arms: usize, body: usize, sites: usize) -> ScalarProject {
        let modules = (0..modules)
            .map(|ordinal| {
                let target = source((ordinal + 1) % modules);
                let program = program(arms, body, sites, ordinal, Some(&target));
                let ScalarItem::Namespace(namespace) = &program.items[0] else {
                    panic!("peer namespace");
                };
                let binding = ScalarNamespaceBinding {
                    binding: namespace.binding.clone(),
                    target,
                    span: namespace.span,
                };
                ScalarModule::from_program(program, vec![binding])
            })
            .collect::<Vec<_>>();
        let order = modules.iter().map(|module| module.source.clone()).collect();
        ScalarProject::new(modules, order)
    }

    fn source_payload(source: &SourceIdentity, payload: &mut observations::Payload) {
        payload.sources += 1;
        payload.string_bytes +=
            source.project.len() + source.package.len() + source.path.len() + source.revision.len();
    }

    fn name_payload(name: &str, payload: &mut observations::Payload) {
        payload.names += 1;
        payload.string_bytes += name.len();
    }

    fn type_payload(ty: &ScalarType, payload: &mut observations::Payload) {
        let mut pending = vec![ty];
        while let Some(ty) = pending.pop() {
            payload.type_nodes += 1;
            match ty {
                ScalarType::Callable {
                    outputs,
                    parameters,
                } => {
                    pending.extend(outputs.outputs.iter().map(|output| &output.ty));
                    pending.extend(parameters);
                }
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    pending.push(element)
                }
                ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
                    pending.push(inner)
                }
                ScalarType::Struct(id) => source_payload(&id.source, payload),
                ScalarType::Enum(id) => source_payload(&id.source, payload),
                ScalarType::Named { name, .. } => name_payload(name, payload),
                ScalarType::Qualified {
                    receiver, member, ..
                } => {
                    name_payload(receiver, payload);
                    name_payload(member, payload);
                }
                _ => {}
            }
        }
    }

    fn enumerate_payload(environment: &ScalarOverloadEnvironment) -> observations::Payload {
        let mut payload = observations::Payload::default();
        for source in environment.sources.keys() {
            source_payload(source, &mut payload);
        }
        for module in &environment.modules {
            payload.callable_headers += module.callables.generics.len();
            for (name, parameters) in &module.callables.generics {
                name_payload(name, &mut payload);
                payload.ordinary_parameters += parameters.len();
                for parameter in parameters {
                    name_payload(&parameter.name, &mut payload);
                }
            }
            payload.extern_bindings += module.callables.externs.len();
            for (binding, members) in &module.callables.externs {
                name_payload(binding, &mut payload);
                payload.extern_members += members.len();
                for (name, member) in members {
                    name_payload(name, &mut payload);
                    type_payload(&member.signature, &mut payload);
                }
            }
            source_payload(&module.source, &mut payload);
            for (name, ty) in &module.scope {
                name_payload(name, &mut payload);
                type_payload(ty, &mut payload);
            }
            for declaration in module.declarations.values() {
                name_payload(&declaration.name, &mut payload);
                payload.headers += 1;
                payload.arms += declaration.header.arms.len();
                for arm in &declaration.header.arms {
                    type_payload(&arm.signature, &mut payload);
                    payload.generics += arm.generic_parameters.len();
                    for generic in &arm.generic_parameters {
                        name_payload(&generic.name, &mut payload);
                    }
                }
            }
            for name in module.first_overloads.keys() {
                name_payload(name, &mut payload);
            }
            for namespace in &module.namespaces {
                name_payload(&namespace.binding, &mut payload);
                source_payload(&namespace.target, &mut payload);
            }
        }
        payload
    }

    fn assert_module_membership(
        header: &ScalarOverloadModule,
        source: &SourceIdentity,
        items: &[ScalarItem],
    ) {
        assert_eq!(&header.source, source);
        let mut generics = BTreeMap::new();
        let mut externs =
            BTreeMap::<&str, BTreeMap<&str, (&ScalarExternFunction, bool, bool)>>::new();
        for item in items {
            match item {
                ScalarItem::Function(function) => {
                    generics
                        .entry(function.name.as_str())
                        .or_insert(&function.generic_parameters);
                }
                ScalarItem::Extern(declaration) => {
                    let first = !externs.contains_key(declaration.binding.as_str());
                    let members = externs.entry(declaration.binding.as_str()).or_default();
                    for function in &declaration.functions {
                        if let Some(member) = members.get_mut(function.name.as_str()) {
                            member.2 |= function.unsafe_marker;
                        } else {
                            members.insert(
                                function.name.as_str(),
                                (function, first, function.unsafe_marker),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        assert_eq!(header.callables.generics.len(), generics.len());
        for (name, parameters) in generics {
            assert_eq!(&header.callables.generics[name], parameters);
        }
        assert_eq!(header.callables.externs.len(), externs.len());
        for (binding, members) in externs {
            assert_eq!(header.callables.externs[binding].len(), members.len());
            for (name, (function, first, any_unsafe)) in members {
                let member = &header.callables.externs[binding][name];
                assert_eq!(member.signature, function.signature);
                assert_eq!(member.first_declaration, first);
                assert_eq!(member.unsafe_marker, function.unsafe_marker);
                assert_eq!(member.any_unsafe, any_unsafe);
            }
        }
        let declarations = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                ScalarItem::Function(function) if !function.overload_arms.is_empty() => {
                    Some((index, function))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(header.declarations.len(), declarations.len());
        for (position, function) in declarations {
            let declaration = header
                .declarations
                .get(&position)
                .expect("complete declaration identity");
            assert_eq!(declaration.name, function.name);
            assert_eq!(declaration.header.arms.len(), function.overload_arms.len());
            for (arm, original) in declaration.header.arms.iter().zip(&function.overload_arms) {
                assert_eq!(arm.signature, original.signature);
                assert_eq!(arm.generic_parameters, original.generic_parameters);
            }
        }
    }

    fn assert_zero_recording_copies(counts: &observations::Counts) {
        assert_eq!(counts.program_copies, 0);
        assert_eq!(counts.module_copies, 0);
        assert_eq!(counts.struct_copies, 0);
        assert_eq!(counts.cfg_copies, 0);
        assert_eq!(counts.copied_context_payload_units, 0);
        assert_eq!(counts.construction_program_copies, 0);
        assert_eq!(counts.construction_module_copies, 0);
        assert_eq!(counts.construction_struct_copies, 0);
        assert_eq!(counts.construction_cfg_copies, 0);
        assert_eq!(counts.construction_payload_units, 0);
    }

    fn assert_selections(items: &[ScalarItem], arms: usize, sites: usize, peer: bool) {
        let mut calls = 0;
        for item in items {
            let ScalarItem::Binding(binding) = item else {
                continue;
            };
            let ScalarExpression::Call {
                receiver,
                overload_selection,
                ..
            } = &binding.value
            else {
                continue;
            };
            assert_eq!(receiver.as_deref(), if peer { Some("peer") } else { None });
            let selection = overload_selection
                .as_ref()
                .expect("recorded call selection");
            if binding.name.starts_with("result") {
                assert_eq!(selection.arm_index, 0);
                assert!(selection.substitutions.is_empty());
            } else {
                assert!(binding.name.starts_with("copied"));
                assert_eq!(selection.arm_index, arms - 1);
                assert_eq!(
                    selection.substitutions,
                    BTreeMap::from([("T".to_owned(), ScalarType::U8)])
                );
            }
            calls += 1;
        }
        assert_eq!(calls, sites * 2);
    }

    #[test]
    fn callable_metadata_preserves_first_empty_generics_and_duplicate_extern_predicates() {
        let parsed = parse_source(source(0), "%%start\ni32(i32) identity = fn(value) { value };\nenv = extern wasm \"helper\" { i32() fetch; };\n%%end".into(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let result = derive_scalar_program(&parsed.result);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let mut items = result.program.items;
        let ScalarItem::Function(first) = &items[0] else {
            panic!("function")
        };
        let mut generic = first.clone();
        generic.generic_parameters = vec![
            ScalarGenericParameter {
                name: "Z".into(),
                span: ByteSpan::new(1, 2),
            },
            ScalarGenericParameter {
                name: "A".into(),
                span: ByteSpan::new(3, 4),
            },
        ];
        let mut ordered = generic.clone();
        ordered.name = "ordered".into();
        let expected_parameters = ordered.generic_parameters.clone();
        items.push(ScalarItem::Function(generic));
        items.push(ScalarItem::Function(ordered));
        let ScalarItem::Extern(first) = &items[1] else {
            panic!("extern")
        };
        let mut duplicate = first.clone();
        duplicate.functions[0].unsafe_marker = true;
        duplicate.functions[0].signature = ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: vec![ScalarOutput {
                    ty: ScalarType::Bool,
                    span: ByteSpan::new(5, 6),
                }],
                span: ByteSpan::new(5, 6),
            },
            parameters: Vec::new(),
        };
        let mut missing = first.clone();
        missing.binding = "empty".into();
        missing.functions.clear();
        let mut late = duplicate.clone();
        late.binding = "empty".into();
        late.functions[0].name = "_private".into();
        items.extend([
            ScalarItem::Extern(duplicate),
            ScalarItem::Extern(missing),
            ScalarItem::Extern(late),
        ]);
        let scope = observations::Scope::new();
        let environment = ScalarOverloadEnvironment::single(&source(0), &items);
        let view = environment.typing(0);
        assert_eq!(view.generics("identity"), Some([].as_slice()));
        assert_eq!(
            view.generics("ordered"),
            Some(expected_parameters.as_slice())
        );
        assert_eq!(view.generics("missing"), None);
        let fetch = view.extern_member(Some("env"), "fetch").unwrap();
        assert!(fetch.first_declaration);
        assert!(!fetch.unsafe_marker);
        assert!(fetch.any_unsafe);
        assert!(
            matches!(&fetch.signature, ScalarType::Callable { outputs, .. } if outputs.outputs[0].ty == ScalarType::I32)
        );
        let private = view.extern_member(Some("empty"), "_private").unwrap();
        assert!(!private.first_declaration);
        assert!(private.unsafe_marker);
        assert!(view.extern_member(None, "fetch").is_none());
        assert_module_membership(&environment.modules[0], &source(0), &items);
        let counts = scope.counts();
        assert_eq!(counts.contexts, 1);
        let mut copied_payload = enumerate_payload(&environment);
        let ScalarItem::Function(overwritten_scope_entry) = &items[0] else {
            panic!("first scope entry")
        };
        name_payload(&overwritten_scope_entry.name, &mut copied_payload);
        type_payload(&overwritten_scope_entry.signature, &mut copied_payload);
        assert_eq!(counts.snapshot, copied_payload);
        assert_eq!(counts.snapshot.callable_headers, 2);
        assert_eq!(counts.snapshot.ordinary_parameters, 2);
        assert_eq!(counts.snapshot.extern_bindings, 2);
        assert_eq!(counts.snapshot.extern_members, 2);
        assert_eq!(counts.projections, observations::Payload::default());
        assert_zero_recording_copies(&counts);
    }

    #[test]
    fn callable_metadata_payload_is_body_independent_and_imports_select_first_complete_source() {
        let mut baseline = None;
        for body in [4, 8, 16] {
            let parsed = parse_source(source(0), format!("%%start\ni32(i32) identity = fn(value) {{ {} value }};\nenv = extern wasm \"helper\" {{ i32() fetch; }};\n%%end", "value; ".repeat(body)), &[]);
            assert!(parsed.diagnostics.is_empty());
            let result = derive_scalar_program(&parsed.result);
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            let mut first = ScalarModule::from_program(result.program, Vec::new());
            let mut second = first.clone();
            let ScalarItem::Function(function) = &mut first.items[0] else {
                panic!("first")
            };
            function.generic_parameters = vec![ScalarGenericParameter {
                name: "First".into(),
                span: ByteSpan::new(1, 2),
            }];
            let ScalarItem::Function(function) = &mut second.items[0] else {
                panic!("second")
            };
            function.generic_parameters = vec![ScalarGenericParameter {
                name: "Second".into(),
                span: ByteSpan::new(3, 4),
            }];
            let modules = [first, second];
            let scope = observations::Scope::new();
            let environment = ScalarOverloadEnvironment::project(&modules);
            let current = environment.typing(1);
            assert_eq!(current.generics("identity").unwrap()[0].name, "Second");
            let imported = current.imported(&modules[1].source);
            assert_eq!(imported.generics("identity").unwrap()[0].name, "First");
            assert!(std::ptr::eq(
                imported.generics("identity").unwrap(),
                environment.typing(0).generics("identity").unwrap()
            ));
            for (header, module) in environment.modules.iter().zip(&modules) {
                assert_module_membership(header, &module.source, &module.items);
            }
            let counts = scope.counts();
            assert_eq!(counts.contexts, 1);
            assert_eq!(counts.snapshot, enumerate_payload(&environment));
            assert_eq!(counts.snapshot.callable_headers, 2);
            assert_eq!(counts.snapshot.ordinary_parameters, 2);
            assert_eq!(counts.snapshot.extern_members, 2);
            assert_zero_recording_copies(&counts);
            if let Some(expected) = &baseline {
                assert_eq!(&counts.snapshot, expected);
            } else {
                baseline = Some(counts.snapshot);
            }
        }
    }

    #[test]
    fn snapshot_enumerates_all_ordered_header_payload_and_borrows_lookup_results() {
        for modules in [4, 8, 16] {
            for arms in [4, 8, 16] {
                let mut baseline = None;
                for body in [4, 8, 16] {
                    let mut project = project(modules, arms, body, 4);
                    let scope = observations::Scope::new();
                    let environment = ScalarOverloadEnvironment::project(&project.modules);
                    {
                        assert_eq!(environment.modules.len(), modules);
                        assert_eq!(environment.sources.len(), modules);
                        let mut ids = HashSet::new();
                        for (ordinal, module) in project.modules.iter().enumerate() {
                            let header = &environment.modules[ordinal];
                            assert_module_membership(header, &module.source, &module.items);
                            assert_eq!(header.scope, module.members);
                            assert_eq!(environment.sources.get(&module.source), Some(&ordinal));
                            assert_eq!(header.namespaces.len(), module.namespace_bindings.len());
                            for (namespace, original) in
                                header.namespaces.iter().zip(&module.namespace_bindings)
                            {
                                assert_eq!(namespace.binding, original.binding);
                                assert_eq!(namespace.target, original.target);
                            }
                            let lookup = environment.lookup(ordinal);
                            assert_eq!(lookup.len(), 2);
                            let (&position, local) = header.declarations.first_key_value().unwrap();
                            assert!(ids.insert((header.source.clone(), position)));
                            assert!(std::ptr::eq(lookup["choose"], &local.header));
                            let target = &environment.modules[(ordinal + 1) % modules];
                            let imported = target.declarations.first_key_value().unwrap().1;
                            assert!(std::ptr::eq(lookup["peer.choose"], &imported.header));
                        }
                        assert_eq!(ids.len(), modules);
                        assert_eq!(scope.counts().snapshot, enumerate_payload(&environment));
                    }
                    let counts = scope.counts();
                    assert_eq!(counts.contexts, 1);
                    assert_eq!(counts.modules, modules);
                    assert_eq!(counts.snapshot.headers, modules);
                    assert_eq!(counts.snapshot.arms, modules * arms);
                    assert_eq!(counts.snapshot.generics, modules);
                    assert_eq!(counts.projections, observations::Payload::default());
                    assert_eq!(counts.lookups, modules);
                    assert_eq!(counts.lookup_entries, modules * 2);
                    assert_zero_recording_copies(&counts);
                    if let Some(previous) = &baseline {
                        assert_eq!(&counts, previous);
                    }
                    baseline = Some(counts.clone());
                    drop(scope);
                    let scope = observations::Scope::new();
                    record_project_overload_selections(&mut project, &environment);
                    for module in &project.modules {
                        assert_selections(&module.items, arms, 4, true);
                    }
                    let mut expected = counts;
                    expected.contexts = 0;
                    expected.modules = 0;
                    expected.snapshot = observations::Payload::default();
                    expected.recorders = 1;
                    expected.selection_writes = modules * 8;
                    expected.substitution_slots = modules * 4;
                    expected.candidate_calls = modules * 8;
                    assert_eq!(scope.counts(), expected);
                }
            }
        }
    }

    #[test]
    fn recorders_reuse_one_header_per_definition_across_growing_site_counts() {
        for sites in [4, 8, 16] {
            let mut program = program(4, 4, sites, 0, None);
            let scope = observations::Scope::new();
            let environment = ScalarOverloadEnvironment::single(&program.source, &program.items);
            record_overload_selections(&mut program, &environment);
            assert_selections(&program.items, 4, sites, false);
            let counts = scope.counts();
            assert_eq!(counts.contexts, 1);
            assert_eq!(counts.snapshot.headers, 1);
            assert_eq!(counts.snapshot.arms, 4);
            assert_eq!(counts.snapshot.generics, 1);
            assert_eq!(counts.lookups, 1);
            assert_eq!(counts.lookup_entries, 1);
            assert_eq!(counts.selection_writes, sites * 2);
            assert_eq!(counts.substitution_slots, sites);
            assert_eq!(counts.projections, observations::Payload::default());
            assert_zero_recording_copies(&counts);
            drop(scope);
            let mut project = project(4, 4, 4, sites);
            let scope = observations::Scope::new();
            let environment = ScalarOverloadEnvironment::project(&project.modules);
            record_project_overload_selections(&mut project, &environment);
            for module in &project.modules {
                assert_selections(&module.items, 4, sites, true);
            }
            let counts = scope.counts();
            assert_eq!(counts.contexts, 1);
            assert_eq!(counts.snapshot.headers, 4);
            assert_eq!(counts.snapshot.arms, 16);
            assert_eq!(counts.snapshot.generics, 4);
            assert_eq!(counts.lookups, 4);
            assert_eq!(counts.lookup_entries, 8);
            assert_eq!(counts.selection_writes, sites * 8);
            assert_eq!(counts.substitution_slots, sites * 4);
            assert_eq!(counts.projections, observations::Payload::default());
            assert_zero_recording_copies(&counts);
        }
    }

    #[test]
    fn lookup_preserves_private_local_and_ordered_import_alias_overwrites() {
        let parse = |ordinal, text: &str| {
            let parsed = parse_source(source(ordinal), text.to_owned(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            ScalarModule::from_program(derive_scalar_program(&parsed.result).program, Vec::new())
        };
        let first = parse(0, "%%start\npick = overload { u64(u64) => fn(value) { value }; };\npick = overload { u8(u8) => fn(value) { value }; };\nonly_first = overload { u64(u64) => fn(value) { value }; };\n_secret = overload { u64(u64) => fn(value) { value }; };\n%%end");
        let second = parse(
            1,
            "%%start\npick = overload { bool(bool) => fn(value) { value }; };\n%%end",
        );
        let mut local = parse(2, "%%start\npick = overload { char(char) => fn(value) { value }; };\n_secret = overload { u8(u8) => fn(value) { value }; };\n%%end");
        local.namespace_bindings = vec![
            ScalarNamespaceBinding {
                binding: "alias".into(),
                target: first.source.clone(),
                span: ByteSpan::new(1, 2),
            },
            ScalarNamespaceBinding {
                binding: "alias".into(),
                target: second.source.clone(),
                span: ByteSpan::new(3, 4),
            },
            ScalarNamespaceBinding {
                binding: "missing".into(),
                target: source(99),
                span: ByteSpan::new(5, 6),
            },
        ];
        let modules = vec![first, second, local];
        let scope = observations::Scope::new();
        {
            let environment = ScalarOverloadEnvironment::project(&modules);
            assert!(std::ptr::eq(
                environment.first(&modules[0].source, "pick").unwrap(),
                &environment.modules[0].declarations[&0].header
            ));
            assert!(std::ptr::eq(
                environment
                    .for_call(&modules[2].source, Some("alias"), "pick")
                    .unwrap(),
                &environment.modules[0].declarations[&0].header
            ));
            let first_lookup = environment.lookup(0);
            assert!(std::ptr::eq(
                first_lookup["pick"],
                &environment.modules[0].declarations[&1].header
            ));
            let lookup = environment.lookup(2);
            assert_eq!(lookup.len(), 4);
            assert!(std::ptr::eq(
                lookup["pick"],
                &environment.modules[2].declarations[&0].header
            ));
            assert!(std::ptr::eq(
                lookup["_secret"],
                &environment.modules[2].declarations[&1].header
            ));
            assert!(std::ptr::eq(
                lookup["alias.pick"],
                &environment.modules[1].declarations[&0].header
            ));
            assert!(std::ptr::eq(
                lookup["alias.only_first"],
                &environment.modules[0].declarations[&2].header
            ));
            assert!(!lookup.contains_key("alias._secret"));
            assert!(!lookup.contains_key("missing.pick"));
        }
        assert_eq!(scope.counts().snapshot.headers, 7);
        assert_zero_recording_copies(&scope.counts());
    }

    #[test]
    fn explicit_projection_preserves_ordered_generic_names_spans_and_signatures() {
        let mut program = program(4, 4, 4, 0, None);
        let ScalarItem::Function(function) = &mut program.items[0] else {
            panic!("overload");
        };
        function.overload_arms[3].generic_parameters = vec![
            ScalarGenericParameter {
                name: "Z".into(),
                span: ByteSpan::new(11, 12),
            },
            ScalarGenericParameter {
                name: "T".into(),
                span: ByteSpan::new(21, 22),
            },
            ScalarGenericParameter {
                name: "A".into(),
                span: ByteSpan::new(31, 32),
            },
        ];
        let scope = observations::Scope::new();
        let copies = clone_observations::Scope::new();
        let header = ScalarOverloadHeader::from_function(function);
        for (arm, original) in header.arms.iter().zip(&function.overload_arms) {
            assert_eq!(arm.signature, original.signature);
            assert_eq!(arm.generic_parameters, original.generic_parameters);
        }
        assert_eq!(scope.counts().projections.headers, 1);
        assert_eq!(scope.counts().projections.arms, 4);
        assert_eq!(scope.counts().projections.generics, 3);
        assert_eq!(scope.counts().snapshot, observations::Payload::default());
        assert_eq!(copies.counts(), clone_observations::Counts::default());
    }

    #[test]
    fn public_derivation_and_project_build_keep_recording_scopes_body_free() {
        let parsed = parse_source(source(0), text(4, 4, 4, None), &[]);
        let outside = clone_observations::Scope::new();
        let scope = observations::Scope::new();
        let validation = derive_scalar_program(&parsed.result);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        assert_selections(&validation.program.items, 4, 4, false);
        let counts = scope.counts();
        assert_eq!(counts.contexts, 1);
        assert_eq!(counts.snapshot.headers, 1);
        assert_eq!(counts.projections, observations::Payload::default());
        assert_eq!(counts.recorders, 1);
        assert_zero_recording_copies(&counts);
        assert_eq!(outside.counts().program_copies, 0);
        assert_eq!(outside.counts().module_copies, 0);
        assert_eq!(outside.counts().cfg_copies, 0);
        drop(scope);
        drop(outside);
        let project = project(2, 4, 4, 4);
        let outside = clone_observations::Scope::new();
        let scope = observations::Scope::new();
        let validation = validate_scalar_project(project);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        for module in &validation.project.modules {
            assert_selections(&module.items, 4, 4, true);
        }
        let counts = scope.counts();
        assert_eq!(counts.contexts, 1);
        assert_eq!(counts.snapshot.headers, 2);
        assert_eq!(counts.projections, observations::Payload::default());
        assert_eq!(counts.recorders, 1);
        assert_zero_recording_copies(&counts);
        assert_eq!(outside.counts().module_copies, 0);
        assert_eq!(outside.counts().cfg_copies, 0);
    }

    #[test]
    fn public_callsites_reuse_fixed_headers_and_account_candidate_and_result_growth() {
        let mut baselines = BTreeMap::new();
        for body in [4, 8, 16] {
            let mut previous_single = 0;
            let mut previous_project = 0;
            for sites in [4, 8, 16] {
                let parsed = parse_source(source(0), text(4, body, sites, None), &[]);
                let scope = observations::Scope::new();
                let single = derive_scalar_program(&parsed.result);
                assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
                assert_selections(&single.program.items, 4, sites, false);
                let counts = scope.counts();
                assert_eq!(counts.contexts, 1);
                assert_eq!(counts.recorders, 1);
                assert_eq!(counts.snapshot.headers, 1);
                assert_eq!(counts.snapshot.arms, 4);
                assert_eq!(counts.projections, observations::Payload::default());
                assert_eq!(counts.selection_writes, sites * 2);
                assert_eq!(counts.substitution_slots, sites);
                assert!(counts.candidate_calls > counts.selection_writes);
                assert!(counts.candidate_calls > previous_single);
                previous_single = counts.candidate_calls;
                assert_zero_recording_copies(&counts);
                if body == 4 {
                    baselines.insert((false, sites), counts.snapshot);
                } else {
                    assert_eq!(&counts.snapshot, &baselines[&(false, sites)]);
                }
                drop(scope);

                let inputs = project(2, 4, body, sites);
                let scope = observations::Scope::new();
                let validation = validate_scalar_project(inputs);
                assert!(
                    validation.diagnostics.is_empty(),
                    "{:?}",
                    validation.diagnostics
                );
                for module in &validation.project.modules {
                    assert_selections(&module.items, 4, sites, true);
                }
                let counts = scope.counts();
                assert_eq!(counts.contexts, 1);
                assert_eq!(counts.recorders, 1);
                assert_eq!(counts.snapshot.headers, 2);
                assert_eq!(counts.snapshot.arms, 8);
                assert_eq!(counts.projections, observations::Payload::default());
                assert_eq!(counts.selection_writes, sites * 4);
                assert_eq!(counts.substitution_slots, sites * 2);
                assert!(counts.candidate_calls > counts.selection_writes);
                assert!(counts.candidate_calls > previous_project);
                previous_project = counts.candidate_calls;
                assert_zero_recording_copies(&counts);
                if body == 4 {
                    baselines.insert((true, sites), counts.snapshot);
                } else {
                    assert_eq!(&counts.snapshot, &baselines[&(true, sites)]);
                }
            }
        }
    }

    #[test]
    fn first_overload_index_ignores_earlier_ordinary_functions() {
        let parsed = parse_source(source(0), "%%start\nu64(u64) pick = fn(value) { value };\npick = overload { bool(bool) => fn(value) { value }; };\n%%end".into(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let program = derive_scalar_program(&parsed.result).program;
        let scope = observations::Scope::new();
        let environment = ScalarOverloadEnvironment::single(&program.source, &program.items);
        let header = environment.first(&program.source, "pick").unwrap();
        assert_eq!(environment.modules[0].first_overloads["pick"], 1);
        assert!(std::ptr::eq(
            header,
            &environment.modules[0].declarations[&1].header
        ));
        let (callable, selection) = resolve_overload_candidate(
            header,
            &[ScalarOverloadArgument::Typed(ScalarType::Bool)],
            Some(&ScalarType::Bool),
        )
        .unwrap();
        assert_eq!(selection.arm_index, 0);
        assert!(
            matches!(callable, ScalarType::Callable { parameters, .. } if parameters == [ScalarType::Bool])
        );
        assert_eq!(scope.counts().snapshot.headers, 1);
        assert_eq!(scope.counts().projections, observations::Payload::default());
    }
}

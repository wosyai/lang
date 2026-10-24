use super::*;

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ScalarOverloadArmHeader {
    pub signature: ScalarType,
    pub generic_parameters: Vec<ScalarGenericParameter>,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ScalarOverloadHeader {
    pub arms: Vec<ScalarOverloadArmHeader>,
}

impl ScalarOverloadHeader {
    pub(super) fn from_function(function: &ScalarFunction) -> Self {
        #[cfg(test)]
        observations::record_header(function);
        Self {
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
    namespaces: Vec<ScalarOverloadNamespace>,
}

struct ScalarOverloadEnvironment {
    modules: Vec<ScalarOverloadModule>,
    sources: HashMap<SourceIdentity, usize>,
}

impl ScalarOverloadEnvironment {
    fn single(source: &SourceIdentity, items: &[ScalarItem]) -> Self {
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
        let module = ScalarOverloadModule {
            source: copy_source(source),
            scope,
            declarations: overload_declarations(items),
            namespaces: Vec::new(),
        };
        Self {
            sources: HashMap::from([(copy_source(&module.source), 0)]),
            modules: vec![module],
        }
    }

    fn project(modules: &[ScalarModule]) -> Self {
        #[cfg(test)]
        observations::record_context(modules.len());
        let mut sources = HashMap::new();
        let modules = modules
            .iter()
            .enumerate()
            .map(|(ordinal, module)| {
                let header = ScalarOverloadModule {
                    source: copy_source(&module.source),
                    scope: module
                        .members
                        .iter()
                        .map(|(name, ty)| (copy_name(name), copy_type(ty)))
                        .collect(),
                    declarations: overload_declarations(&module.items),
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

fn overload_declarations(items: &[ScalarItem]) -> BTreeMap<usize, ScalarOverloadDeclaration> {
    items
        .iter()
        .enumerate()
        .filter_map(|(position, item)| match item {
            ScalarItem::Function(function) if !function.overload_arms.is_empty() => Some((
                position,
                ScalarOverloadDeclaration {
                    name: copy_name(&function.name),
                    header: ScalarOverloadHeader::from_function(function),
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

pub(super) fn record_overload_selections(program: &mut ScalarProgram) {
    #[cfg(test)]
    let _recording = observations::RecordingScope::new();
    let environment = ScalarOverloadEnvironment::single(&program.source, &program.items);
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
        ScalarBlockItem::While(while_expression) => {
            record_block_overload_selections(&mut while_expression.body, &scope, overloads)
        }
    }
}

pub(super) fn record_project_overload_selections(project: &mut ScalarProject) {
    #[cfg(test)]
    let _recording = observations::RecordingScope::new();
    let environment = ScalarOverloadEnvironment::project(&project.modules);
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
mod observations {
    use super::super::layout::observations as clone_observations;
    use super::*;

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub(super) struct Payload {
        pub headers: usize,
        pub arms: usize,
        pub generics: usize,
        pub type_nodes: usize,
        pub names: usize,
        pub sources: usize,
        pub string_bytes: usize,
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub(super) struct Counts {
        pub contexts: usize,
        pub modules: usize,
        pub snapshot: Payload,
        pub projections: Payload,
        pub lookups: usize,
        pub lookup_entries: usize,
        pub lookup_key_bytes: usize,
        pub selection_writes: usize,
        pub substitution_slots: usize,
        pub program_copies: usize,
        pub module_copies: usize,
        pub struct_copies: usize,
        pub cfg_copies: usize,
        pub copied_context_payload_units: usize,
    }

    struct State {
        counts: Counts,
        recording: bool,
    }
    thread_local! {
        static ACTIVE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    pub(super) struct Scope {
        previous: Option<State>,
    }
    impl Scope {
        pub(super) fn new() -> Self {
            Self {
                previous: ACTIVE.with(|active| {
                    active.replace(Some(State {
                        counts: Counts::default(),
                        recording: false,
                    }))
                }),
            }
        }
        pub(super) fn counts(&self) -> Counts {
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
    }
    impl RecordingScope {
        pub(super) fn new() -> Self {
            let active = ACTIVE.with(|active| {
                if let Some(state) = active.borrow_mut().as_mut() {
                    assert!(!state.recording);
                    state.recording = true;
                    true
                } else {
                    false
                }
            });
            Self {
                clones: active.then(clone_observations::Scope::new),
            }
        }
    }
    impl Drop for RecordingScope {
        fn drop(&mut self) {
            if let Some(scope) = &self.clones {
                let copies = scope.counts();
                record(|counts| {
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

    pub(super) fn record_context(modules: usize) {
        record(|counts| {
            counts.contexts += 1;
            counts.modules += modules;
        });
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
    fn snapshot_enumerates_all_ordered_header_payload_and_borrows_lookup_results() {
        for modules in [4, 8, 16] {
            for arms in [4, 8, 16] {
                let mut baseline = None;
                for body in [4, 8, 16] {
                    let mut project = project(modules, arms, body, 4);
                    let scope = observations::Scope::new();
                    {
                        let _recording = observations::RecordingScope::new();
                        let environment = ScalarOverloadEnvironment::project(&project.modules);
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
                    record_project_overload_selections(&mut project);
                    for module in &project.modules {
                        assert_selections(&module.items, arms, 4, true);
                    }
                    let mut expected = counts;
                    expected.selection_writes = modules * 8;
                    expected.substitution_slots = modules * 4;
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
            record_overload_selections(&mut program);
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
            record_project_overload_selections(&mut project);
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
            let _recording = observations::RecordingScope::new();
            let environment = ScalarOverloadEnvironment::project(&modules);
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
        assert!(counts.projections.headers > 0);
        assert_zero_recording_copies(&counts);
        assert!(outside.counts().program_copies > 0);
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
        assert!(counts.projections.headers > 0);
        assert_zero_recording_copies(&counts);
        assert!(outside.counts().module_copies > 0);
        assert!(outside.counts().cfg_copies > 0);
    }
}

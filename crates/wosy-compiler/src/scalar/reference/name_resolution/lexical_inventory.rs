use super::*;
mod direct_storage;
pub(in crate::scalar) use direct_storage::{direct_constructors, allocation_constructors};
mod original_operand;
use original_operand::*;
mod checked_pointer_cast;
pub(in crate::scalar) use checked_pointer_cast::checked_pointer_cast_constructors;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Declaration {
    binding: ReferenceBindingId,
    declaration: SourceNameDeclarationRef,
    original: SourceNameDeclarationRef,
    role: SourceNameResolutionRole,
    type_source: DeclarationTypeSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DeclarationTypeSource {
    Declared(ScalarType),
    Assignment { occurrence: ScalarAssignmentOccurrence, ordinal: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NameSource {
    Value(Declaration),
    Evaluate,
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExpectedName {
    expression: ScalarExpression,
    occurrence: SourceSpan,
    block: ByteSpan,
    source: NameSource,
}

struct CaptureScope {
    occurrence: SourceSpan,
    declarations: BTreeMap<String, Declaration>,
}

struct Inventory {
    names: Vec<ExpectedName>,
    captures: Vec<CaptureScope>,
    nulls: Vec<ExpectedName>,
    raw_primary_nulls: Vec<SourceSpan>,
    direct_addresses: Vec<direct_storage::ExpectedAddress>,
    rebind_addresses: Vec<direct_storage::ExpectedRebind>,
    checked_pointer_casts: Vec<checked_pointer_cast::ExpectedCheckedPointerCast>,
}

struct Walker<'a> {
    owner: ProducerCfgOwnerId,
    identity: SourceIdentity,
    function_span: ByteSpan,
    locals: BTreeMap<String, Declaration>,
    globals: BTreeMap<String, Declaration>,
    module_names: HashSet<String>,
    structs: &'a [ScalarStruct],
    inventory: Inventory,
    next_rebind_association: usize,
}

fn invalid(occurrence: &SourceSpan) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext { source_span: occurrence.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence }
}

fn identity(owner: &ProducerCfgOwnerId) -> &SourceIdentity {
    match owner {
        ProducerCfgOwnerId::Module(source) | ProducerCfgOwnerId::Function { source, .. } => source,
        ProducerCfgOwnerId::Literal { enclosing, .. } => identity(enclosing),
    }
}

fn original(context: &SourceNameResolutionContext, owner: &ProducerCfgOwnerId) -> Result<SourceNameResolutionContextRoot, ReferenceAnalysisError> {
    if context.owner == *owner {
        return Ok(SourceNameResolutionContextRoot { owner: context.owner.clone(), root: context.root.clone(), globals: context.globals.clone(), parents: context.parents.clone() });
    }
    let roots = context.ancestors.iter().filter(|root| root.owner == *owner).collect::<Vec<_>>();
    let [root] = roots.as_slice() else { return Err(invalid(&SourceSpan::new(identity(owner).clone(), ByteSpan::new(0, 0)))); };
    Ok((*root).clone())
}

fn module_names(context: &SourceNameResolutionContext) -> HashSet<String> {
    std::iter::once(&context.root).chain(context.ancestors.iter().map(|root| &root.root))
        .filter_map(|root| match root { SourceNameResolutionRoot::Module { items, .. } => Some(items), _ => None }).flatten()
        .filter_map(|item| match item {
            SourceNameModuleItem::Binding(binding) => Some(binding.name.clone()),
            SourceNameModuleItem::Function(function) => Some(function.name.clone()),
            SourceNameModuleItem::Namespace(namespace) => Some(namespace.binding.clone()),
            SourceNameModuleItem::Extern(external) => Some(external.binding.clone()),
            SourceNameModuleItem::Executable(_) => None,
        }).collect()
}

impl Walker<'_> {
    fn declaration(&self, span: ByteSpan, block: ByteSpan, site: SourceNameDeclarationSite, role: SourceNameResolutionRole, type_source: DeclarationTypeSource) -> Declaration {
        let reference = SourceNameDeclarationRef { owner: self.owner.clone(), site };
        Declaration { binding: ReferenceBindingId { source: self.identity.clone(), function_span: self.function_span,
            declaration_span: span, block_span: block, kind: ReferenceBindingKind::Declared }, original: reference.clone(), declaration: reference, role, type_source }
    }

    fn parameters(&mut self, names: &[String], spans: &[ByteSpan], signature: &ScalarType, block: ByteSpan) -> Result<(), ReferenceAnalysisError> {
        let ScalarType::Callable { parameters, .. } = signature else { return Err(invalid(&SourceSpan::new(self.identity.clone(), self.function_span))); };
        if names.len() != spans.len() || names.len() != parameters.len() { return Err(invalid(&SourceSpan::new(self.identity.clone(), self.function_span))); }
        for (ordinal, (name, span)) in names.iter().zip(spans).enumerate() {
            self.locals.insert(name.clone(), self.declaration(*span, block, SourceNameDeclarationSite::Parameter { ordinal }, SourceNameResolutionRole::Parameter, DeclarationTypeSource::Declared(parameters[ordinal].clone())));
        }
        Ok(())
    }

    fn expression(&mut self, expression: &ScalarExpression, block: ByteSpan) -> Result<(), ReferenceAnalysisError> {
        match expression {
            ScalarExpression::Name { name, span } => {
                let source = if name == "null" { NameSource::Evaluate } else {
                    match self.locals.get(name) {
                        Some(declaration) => NameSource::Value(declaration.clone()),
                        None => match self.globals.get(name) {
                            Some(declaration) => NameSource::Value(declaration.clone()),
                            None if self.module_names.contains(name) => NameSource::Evaluate,
                            None => NameSource::Missing,
                        },
                    }
                };
                let expected = ExpectedName { expression: expression.clone(), occurrence: SourceSpan::new(self.identity.clone(), *span), block, source };
                if name == "null" { self.inventory.nulls.push(expected.clone()); }
                self.inventory.names.push(expected);
            },
            ScalarExpression::FunctionLiteral { span, .. } => self.inventory.captures.push(CaptureScope {
                occurrence: SourceSpan::new(self.identity.clone(), *span), declarations: self.locals.clone(),
            }),
            ScalarExpression::Assignment(assignment) => self.assignment(assignment, block, true)?,
            ScalarExpression::Binary { left, right, .. } => { self.expression(left, block)?; self.expression(right, block)?; },
            ScalarExpression::Unary { operand, .. } => self.expression(operand, block)?,
            ScalarExpression::Call { arguments, receiver, name, .. } => {
                for (ordinal, argument) in arguments.iter().enumerate() {
                    if ordinal == 0 && receiver.as_deref() == Some("core") && CoreOperationId::from_name(name) == Some(CoreOperationId::Rebind)
                        && matches!(argument, ScalarExpression::CheckedAddress { place, .. } if matches!(super::super::reference_root(place), ScalarPlace::Name { .. })) {
                        self.rebind_address(expression, argument, block)?;
                    } else { self.expression(argument, block)?; }
                }
                self.checked_pointer_cast(expression, block)?;
            },
            ScalarExpression::If { condition, then_branch, else_branch, .. } => { self.expression(condition, block)?; self.block(then_branch)?; self.block(else_branch)?; },
            ScalarExpression::UnitIf { condition, then_branch, .. } => { self.expression(condition, block)?; self.block(then_branch)?; },
            ScalarExpression::Block(body) => self.block(body)?,
            ScalarExpression::ArrayLiteral { elements, .. } => { for element in elements { self.expression(element, block)?; } },
            ScalarExpression::StructLiteral { fields, .. } => { for field in fields { self.expression(&field.value, block)?; } },
            ScalarExpression::CheckedAddress { .. } => self.direct_address(expression, block)?,
            ScalarExpression::RawAddress { place, .. }
                | ScalarExpression::Dereference { place, .. } | ScalarExpression::IndexedRead { place, .. } => self.place(place, block)?,
            ScalarExpression::Member { .. } | ScalarExpression::Integer { .. } | ScalarExpression::InvalidInteger { .. }
                | ScalarExpression::Float { .. } | ScalarExpression::InvalidFloat { .. } | ScalarExpression::Boolean { .. }
                | ScalarExpression::Char { .. } | ScalarExpression::Utf8 { .. } => {},
        }
        Ok(())
    }

    fn place(&mut self, place: &ScalarPlace, block: ByteSpan) -> Result<(), ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { .. } => {},
            ScalarPlace::Field { base, .. } => self.place(base, block)?,
            ScalarPlace::Index { base, index, .. } => { self.place(base, block)?; self.expression(index, block)?; },
            ScalarPlace::Dereference { pointer, .. } => self.expression(pointer, block)?,
        }
        Ok(())
    }

    fn receiver(&mut self, binding: &ScalarBinding, ordinal: usize, block: ByteSpan, role: SourceNameResolutionRole) {
        let receiver = &binding.receivers[ordinal];
        let (name, span) = if ordinal == 0 { (&binding.name, binding.name_span) } else { (&receiver.name, receiver.name_span) };
        let ty = if ordinal == 0 { binding.declared_type.clone() } else { receiver.ty.clone() };
        let declaration = self.declaration(span, block, SourceNameDeclarationSite::Receiver { occurrence: (*binding.occurrence).clone(), ordinal }, role, DeclarationTypeSource::Declared(ty));
        self.locals.insert(name.clone(), declaration);
    }

    fn binding(&mut self, binding: &ScalarBinding, block: ByteSpan, role: SourceNameResolutionRole) -> Result<(), ReferenceAnalysisError> {
        if binding.receivers[0].allocation_length.is_none()
            && matches!(binding.declared_type, ScalarType::RawPointer(_))
            && matches!(&binding.value, ScalarExpression::Name { name, .. } if name == "null") {
            self.inventory.raw_primary_nulls.push(SourceSpan::new(self.identity.clone(), expression_span(&binding.value)));
        }
        if let Some(length) = &binding.receivers[0].allocation_length { self.expression(length, block)?; }
        else { self.expression(&binding.value, block)?; }
        self.receiver(binding, 0, block, role.clone());
        for ordinal in 1..binding.receivers.len() {
            if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions { self.expression(&binding.output_values[ordinal].value, block)?; }
            self.receiver(binding, ordinal, block, role.clone());
        }
        Ok(())
    }

    fn assignment(&mut self, assignment: &ScalarAssignment, block: ByteSpan, observed: bool) -> Result<(), ReferenceAnalysisError> {
        for value in &assignment.values { self.expression(value, block)?; }
        for (ordinal, target) in assignment.targets.iter().enumerate() {
            if let ScalarPlace::Name { name, .. } = &target.place {
                if !observed && !self.locals.contains_key(name) && !self.module_names.contains(name) {
                    let declaration = self.declaration(target.target_span, block,
                        SourceNameDeclarationSite::Assignment { occurrence: (*assignment.occurrence).clone(), ordinal }, SourceNameResolutionRole::InferredAssignment,
                        DeclarationTypeSource::Assignment { occurrence: (*assignment.occurrence).clone(), ordinal });
                    self.locals.insert(name.clone(), declaration);
                }
            }
            self.place(&target.place, block)?;
        }
        Ok(())
    }

    fn item(&mut self, item: &ScalarBlockItem, block: ByteSpan) -> Result<(), ReferenceAnalysisError> {
        match item {
            ScalarBlockItem::LocalBinding(binding) => self.binding(binding, block, SourceNameResolutionRole::Local)?,
            ScalarBlockItem::Expression(expression) => self.expression(expression, block)?,
            ScalarBlockItem::Assignment(assignment) => self.assignment(assignment, block, false)?,
            ScalarBlockItem::While(loop_) => { self.expression(&loop_.condition, block)?; self.block(&loop_.body)?; },
        }
        Ok(())
    }

    fn block(&mut self, block: &ScalarBlock) -> Result<(), ReferenceAnalysisError> {
        let saved = self.locals.clone();
        match group_conditional_final_outputs(&self.identity, block)? {
            Some(group) => {
                let prefix = block.items.len() - block.final_output_values.len();
                for item in &block.items[..prefix] { self.item(item, block.span)?; }
                self.expression(&group.expression, block.span)?;
            },
            None => { for item in &block.items { self.item(item, block.span)?; } },
        }
        self.locals = saved;
        Ok(())
    }

    fn module_assignment(&mut self, assignment: &ScalarAssignment, types: &HashMap<ReferenceBindingId, ScalarType>) -> Result<(), ReferenceAnalysisError> {
        let block = ByteSpan::new(0, 0);
        let names = self.locals.iter().map(|(name, declaration)| (name.clone(), declaration.binding.clone())).collect::<BTreeMap<_, _>>();
        let array_outputs = assignment.targets.iter().any(|target| match &target.place {
            ScalarPlace::Name { name, .. } => names.get(name).and_then(|binding| types.get(binding)).is_some_and(|ty| matches!(ty, ScalarType::RuntimeArray { .. } | ScalarType::Array { .. })),
            _ => false,
        });
        if array_outputs || (assignment.values.len() == 1 && assignment.targets.len() > 1 && matches!(assignment.values[0], ScalarExpression::Call { .. })) {
            for value in &assignment.values { self.expression(value, block)?; }
            return Ok(());
        }
        let mut produced = false;
        for (target, value) in assignment.targets.iter().zip(&assignment.values) {
            let ty = super::super::module_assignment_place_type(&target.place, &names, types, self.structs);
            let active = match (&target.place, ty) {
                (ScalarPlace::Name { .. }, Some(ScalarType::CheckedReference { .. } | ScalarType::Callable { .. } | ScalarType::Struct(_))) => true,
                (ScalarPlace::Field { .. }, Some(ScalarType::CheckedReference { .. } | ScalarType::Callable { .. } | ScalarType::RuntimeArray { .. })) => true,
                (ScalarPlace::Index { .. }, Some(ScalarType::Callable { .. })) => true,
                _ => false,
            };
            if active {
                self.expression(value, block)?;
                if !matches!(target.place, ScalarPlace::Name { .. }) { self.place(&target.place, block)?; }
                produced = true;
            }
        }
        if !produced {
            for value in &assignment.values { self.expression(value, block)?; }
            for target in &assignment.targets { self.place(&target.place, block)?; }
        }
        Ok(())
    }

    fn module(&mut self, items: &[SourceNameModuleItem]) -> Result<(), ReferenceAnalysisError> {
        let block = ByteSpan::new(0, 0);
        let mut types = HashMap::new();
        for item in items {
            match item {
                SourceNameModuleItem::Binding(binding) => {
                    if matches!(binding.value, ScalarExpression::Block(_) | ScalarExpression::If { .. }) {
                        self.expression(&binding.value, block)?;
                    } else if matches!(binding.declared_type, ScalarType::CheckedReference { .. }) {
                        if matches!(binding.value, ScalarExpression::CheckedAddress { .. } | ScalarExpression::Name { .. } | ScalarExpression::Call { .. }) { self.expression(&binding.value, block)?; }
                    } else if let Some(length) = &binding.receivers[0].allocation_length { self.expression(length, block)?; }
                    else { self.expression(&binding.value, block)?; }
                    self.receiver(binding, 0, block, SourceNameResolutionRole::Global);
                    types.insert(self.locals[&binding.name].binding.clone(), binding.declared_type.clone());
                    for ordinal in 1..binding.receivers.len() {
                        if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions { self.expression(&binding.output_values[ordinal].value, block)?; }
                        self.receiver(binding, ordinal, block, SourceNameResolutionRole::Global);
                        types.insert(self.locals[&binding.receivers[ordinal].name].binding.clone(), binding.receivers[ordinal].ty.clone());
                    }
                },
                SourceNameModuleItem::Executable(ScalarBlockItem::Assignment(assignment)) => self.module_assignment(assignment, &types)?,
                SourceNameModuleItem::Executable(ScalarBlockItem::Expression(expression)) if matches!(expression, ScalarExpression::Call { .. } | ScalarExpression::Block(_) | ScalarExpression::If { .. } | ScalarExpression::UnitIf { .. }) => self.expression(expression, block)?,
                SourceNameModuleItem::Executable(_) | SourceNameModuleItem::Function(_) | SourceNameModuleItem::Namespace(_) | SourceNameModuleItem::Extern(_) => {},
            }
        }
        Ok(())
    }
}

fn derive(context: &SourceNameResolutionContext, owner: &ProducerCfgOwnerId, structs: &[ScalarStruct], visited: &mut Vec<ProducerCfgOwnerId>) -> Result<Inventory, ReferenceAnalysisError> {
    let root = original(context, owner)?;
    let error = || invalid(&SourceSpan::new(identity(owner).clone(), ByteSpan::new(0, 0)));
    if visited.contains(owner) { return Err(error()); }
    visited.push(owner.clone());
    let function_span = match &root.root { SourceNameResolutionRoot::Named(function) => function.span, SourceNameResolutionRoot::Literal { occurrence, .. } => occurrence.range, SourceNameResolutionRoot::Module { .. } => ByteSpan::new(0, 0) };
    let mut walker = Walker { owner: owner.clone(), identity: identity(owner).clone(), function_span, next_rebind_association: 0, locals: BTreeMap::new(), globals: BTreeMap::new(), module_names: module_names(context), structs, inventory: Inventory { names: Vec::new(), captures: Vec::new(), nulls: Vec::new(), raw_primary_nulls: Vec::new(), direct_addresses: Vec::new(), rebind_addresses: Vec::new(), checked_pointer_casts: Vec::new() } };
    if !matches!(root.root, SourceNameResolutionRoot::Module { .. }) {
        for global in &root.globals {
            let reference = SourceNameDeclarationRef { owner: global.owner.clone(), site: SourceNameDeclarationSite::Receiver { occurrence: (*global.binding.occurrence).clone(), ordinal: 0 } };
            let declaration = Declaration { binding: ReferenceBindingId { source: identity(&global.owner).clone(), function_span: ByteSpan::new(0, 0), declaration_span: global.binding.name_span, block_span: ByteSpan::new(0, 0), kind: ReferenceBindingKind::Declared }, declaration: reference.clone(), original: reference, role: SourceNameResolutionRole::Global, type_source: DeclarationTypeSource::Declared(global.binding.declared_type.clone()) };
            if walker.globals.insert(global.binding.name.clone(), declaration).is_some() { return Err(error()); }
        }
    }
    match &root.root {
        SourceNameResolutionRoot::Named(function) => { walker.parameters(&function.parameters, &function.parameter_spans, &function.signature, function.body.span)?; walker.block(&function.body)?; },
        SourceNameResolutionRoot::Module { items, .. } => walker.module(items)?,
        SourceNameResolutionRoot::Literal { parent, occurrence, signature, parameters, parameter_spans, captures, body } => {
            let parent_inventory = derive(context, &parent.owner, structs, visited)?;
            let scopes = parent_inventory.captures.iter().filter(|scope| scope.occurrence == *occurrence).collect::<Vec<_>>();
            let [scope] = scopes.as_slice() else { return Err(invalid(occurrence)); };
            for (ordinal, capture) in captures.iter().enumerate() {
                let declared = scope.declarations.get(&capture.name).ok_or_else(|| invalid(&capture.use_span))?;
                if declared.binding != capture.binding { return Err(invalid(&capture.use_span)); }
                let declaration = SourceNameDeclarationRef { owner: owner.clone(), site: SourceNameDeclarationSite::Capture { event: parent.clone(), ordinal } };
                walker.locals.insert(capture.name.clone(), Declaration { binding: declared.binding.clone(), declaration, original: declared.original.clone(), role: SourceNameResolutionRole::Capture, type_source: declared.type_source.clone() });
            }
            walker.parameters(parameters, parameter_spans, signature, body.span)?;
            walker.block(body)?;
        },
    }
    visited.pop();
    Ok(walker.inventory)
}

fn expected(context: &SourceNameResolutionContext, structs: &[ScalarStruct]) -> Result<Inventory, ReferenceAnalysisError> {
    derive(context, &context.owner, structs, &mut Vec::new())
}

fn scope_matches(points: &[ReferenceFlowPoint], point: &ReferenceFlowPoint, block: ByteSpan) -> bool {
    points.iter().any(|entry| entry.scope == point.scope && entry.source_span.source == point.source_span.source && entry.source_span.range == block
        && matches!(entry.kind, CfgPointKind::ScopeEnter | CfgPointKind::Entry))
}

fn fact(cfg: &ScalarReferenceCfg, point: &ReferenceFlowPoint, expected: &ExpectedName, declaration: &Declaration) -> Result<SourceNameReadFact, ReferenceAnalysisError> {
    let values = cfg.evaluated_values.values.iter().filter(|value| value.id.point == point.id && value.id.resolved.producer == cfg.producer_owner).collect::<Vec<_>>();
    let [value] = values.as_slice() else { return Err(invalid(&expected.occurrence)); };
    let shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == value.shape).collect::<Vec<_>>();
    let [shape] = shapes.as_slice() else { return Err(invalid(&expected.occurrence)); };
    if point.source_span != expected.occurrence || !matches!(&point.kind, CfgPointKind::Read { binding } if binding == &declaration.binding)
        || value.id.resolved.occurrence != expected.occurrence || value.id.resolved.output != 0 || !value.id.resolved.output_projection.is_empty()
        || shape.id.producer != cfg.producer_owner { return Err(invalid(&expected.occurrence)); }
    Ok(SourceNameReadFact { occurrence: expected.occurrence.clone(), expression: expected.expression.clone(), event: SourceEventId { owner: cfg.producer_owner.clone(), point: point.id },
        value: value.id.clone(), binding: declaration.binding.clone(), declaration: declaration.declaration.clone(), original_declaration: declaration.original.clone(), role: declaration.role.clone() })
}

pub(in crate::scalar::reference) fn record(builder: &mut ReferenceFlowBuilder, expression: &ScalarExpression, point: CfgPointId) -> Result<(), ReferenceAnalysisError> {
    let ScalarExpression::Name { name, span } = expression else { return Ok(()); };
    let actual = &builder.cfg.points[point.0];
    let CfgPointKind::Read { binding } = &actual.kind else {
        if matches!(&actual.kind, CfgPointKind::Evaluate(registered) if registered == expression) { return Ok(()); }
        return Err(invalid(&actual.source_span));
    };
    if builder.binding(name, *span)? != *binding { return Err(invalid(&actual.source_span)); }
    let NameResolutionContext::Complete(context) = &builder.cfg.name_resolution_context else { return Err(invalid(&actual.source_span)); };
    let inventory = expected(context, &builder.structs)?;
    let names = inventory.names.iter().filter(|expected| expected.expression == *expression && expected.occurrence == actual.source_span && scope_matches(&builder.cfg.points, actual, expected.block)).collect::<Vec<_>>();
    let [expected] = names.as_slice() else { return Err(invalid(&actual.source_span)); };
    let NameSource::Value(declaration) = &expected.source else { return Err(invalid(&actual.source_span)); };
    let row = fact(&builder.cfg, actual, expected, declaration)?;
    if builder.cfg.evaluated_values.source_name_reads.iter().any(|known| known.event == row.event || known.value == row.value) { return Err(invalid(&actual.source_span)); }
    builder.cfg.evaluated_values.source_name_reads.push(row);
    Ok(())
}

fn validate_null_inventory(cfg: &ScalarReferenceCfg, inventory: &Inventory, structs: &[ScalarStruct]) -> Result<(), ReferenceAnalysisError> {
    let mut required = Vec::new();
    let mut events = BTreeSet::new();
    let mut visited = Vec::new();
    for expected in &inventory.nulls {
        if visited.contains(expected) { continue; }
        visited.push(expected.clone());
        let points = cfg.initial_points.iter().filter(|point| point.source_span == expected.occurrence
            && matches!(&point.kind, CfgPointKind::Evaluate(expression) if expression == &expected.expression)
            && scope_matches(&cfg.initial_points, point, expected.block)).collect::<Vec<_>>();
        let count = inventory.nulls.iter().filter(|occurrence| *occurrence == expected).count();
        if points.len() != count { return Err(invalid(&expected.occurrence)); }
        for point in points {
            if cfg.initial_points.get(point.id.0) != Some(point) || !events.insert(point.id) { return Err(invalid(&expected.occurrence)); }
            let Some(origins) = &point.possible_origins else { return Err(invalid(&expected.occurrence)); };
            if inventory.raw_primary_nulls.contains(&expected.occurrence) {
                if !origins.is_empty() { return Err(invalid(&expected.occurrence)); }
                continue;
            }
            let original_origins = origins.iter().copied().collect::<Vec<_>>();
            let [origin] = original_origins.as_slice() else { return Err(invalid(&expected.occurrence)); };
            let definition = ReferenceOrigin::Null { span: expected.occurrence.range };
            if cfg.facts.get(origin.0) != Some(&definition)
                || cfg.facts.iter().position(|fact| *fact == definition) != Some(origin.0) { return Err(invalid(&expected.occurrence)); }
            required.push(source_boundary::SourceNullConstructor {
                origin: source_boundary::SourceConstructorOriginRef { owner: cfg.producer_owner.clone(), origin: *origin },
                event: SourceEventId { owner: cfg.producer_owner.clone(), point: point.id },
                source_span: expected.occurrence.clone(), scope: point.scope,
                expression: expected.expression.clone(), definition,
            });
        }
    }
    for point in &cfg.initial_points {
        if matches!(&point.kind, CfgPointKind::Evaluate(ScalarExpression::Name { name, .. }) if name == "null")
            && !events.contains(&point.id) { return Err(invalid(&point.source_span)); }
    }
    required.sort_by_key(|row| row.event.point);
    if cfg.source_null_constructors != required { return Err(invalid(&cfg.initial_points[0].source_span)); }
    source_boundary::checked_parameter_constructor::validate_mixed(cfg, &required, structs)?;
    Ok(())
}

pub(in crate::scalar) fn validate_null_constructors(cfg: &ScalarReferenceCfg, structs: &[ScalarStruct]) -> Result<(), ReferenceAnalysisError> {
    let NameResolutionContext::Complete(context) = &cfg.name_resolution_context else { return Err(invalid(&cfg.initial_points[0].source_span)); };
    if context.owner != cfg.producer_owner { return Err(invalid(&cfg.initial_points[0].source_span)); }
    validate_null_inventory(cfg, &expected(context, structs)?, structs)
}

pub(in crate::scalar) fn validate(cfg: &ScalarReferenceCfg, structs: &[ScalarStruct]) -> Result<(), ReferenceAnalysisError> {
    let NameResolutionContext::Complete(context) = &cfg.name_resolution_context else { return Err(invalid(&cfg.initial_points[0].source_span)); };
    if context.owner != cfg.producer_owner { return Err(invalid(&cfg.initial_points[0].source_span)); }
    let inventory = expected(context, structs)?;
    let mut required = Vec::new();
    for expected in &inventory.names {
        match &expected.source {
            NameSource::Value(declaration) => {
                let points = cfg.initial_points.iter().filter(|point| point.source_span == expected.occurrence && matches!(point.kind, CfgPointKind::Read { .. }) && scope_matches(&cfg.initial_points, point, expected.block)).collect::<Vec<_>>();
                if points.is_empty() { return Err(invalid(&expected.occurrence)); }
                for point in points { required.push(fact(cfg, point, expected, declaration)?); }
            },
            NameSource::Missing => return Err(invalid(&expected.occurrence)),
            NameSource::Evaluate => {
                if !cfg.initial_points.iter().any(|point| point.source_span == expected.occurrence && matches!(&point.kind, CfgPointKind::Evaluate(expression) if expression == &expected.expression) && scope_matches(&cfg.initial_points, point, expected.block)) { return Err(invalid(&expected.occurrence)); }
            },
        }
    }
    if required.len() != cfg.evaluated_values.source_name_reads.len() { return Err(invalid(&cfg.initial_points[0].source_span)); }
    for row in &required {
        if cfg.evaluated_values.source_name_reads.iter().filter(|actual| *actual == row).count() != 1 { return Err(invalid(&row.occurrence)); }
    }
    for point in &cfg.initial_points {
        if matches!(point.kind, CfgPointKind::Read { .. }) && cfg.evaluated_values.values.iter().any(|value| value.id.point == point.id && value.id.resolved.producer == cfg.producer_owner)
            && required.iter().filter(|row| row.event.point == point.id).count() != 1 { return Err(invalid(&point.source_span)); }
    }
    validate_null_inventory(cfg, &inventory, structs)
}

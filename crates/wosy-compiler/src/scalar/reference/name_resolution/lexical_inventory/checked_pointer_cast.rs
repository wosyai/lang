use super::*;
use source_boundary::checked_pointer_cast_constructor::SourceCheckedPointerCastConstructorRef;
use evaluated_value::{RawAddressOperandRole, RawAddressOperandStep};

#[derive(Clone, Debug, Eq, PartialEq)]
enum CastSource {
    Null,
    LexicalBinding(Declaration),
    CallOutput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExpectedCheckedPointerCast {
    call: ScalarExpression,
    raw: ScalarExpression,
    occurrence: SourceSpan,
    block: ByteSpan,
    function_span: ByteSpan,
    source: CastSource,
}

impl Walker<'_> {
    pub(super) fn checked_pointer_cast(&mut self, expression: &ScalarExpression, block: ByteSpan) -> Result<(), ReferenceAnalysisError> {
        let ScalarExpression::Call { receiver, name, type_arguments, arguments, .. } = expression else { return Err(invalid(&SourceSpan::new(self.identity.clone(), expression_span(expression)))); };
        if receiver.as_deref() != Some("core") || CoreOperationId::from_name(name) != Some(CoreOperationId::PointerCast) { return Ok(()); }
        let [destination] = type_arguments.as_slice() else { return Ok(()); };
        if !matches!(destination.ty, ScalarType::CheckedReference { .. }) { return Ok(()); }
        let occurrence = SourceSpan::new(self.identity.clone(), expression_span(expression));
        let [raw] = arguments.as_slice() else { return Err(invalid(&occurrence)); };
        let source = match raw {
            ScalarExpression::Name { name, .. } if name == "null" => CastSource::Null,
            ScalarExpression::Name { name, .. } => CastSource::LexicalBinding(self.cast_declaration(name, &occurrence)?),
            ScalarExpression::RawAddress { place, .. } => match super::super::super::reference_root(place) {
                ScalarPlace::Name { name, .. } => CastSource::LexicalBinding(self.cast_declaration(name, &occurrence)?),
                ScalarPlace::Dereference { .. } => CastSource::CallOutput,
                _ => return Err(invalid(&occurrence)),
            },
            _ => CastSource::CallOutput,
        };
        self.inventory.checked_pointer_casts.push(ExpectedCheckedPointerCast { call: expression.clone(), raw: raw.clone(), occurrence, block, function_span: self.function_span, source });
        Ok(())
    }

    fn cast_declaration(&self, name: &str, occurrence: &SourceSpan) -> Result<Declaration, ReferenceAnalysisError> {
        match self.locals.get(name) {
            Some(declaration) => Ok(declaration.clone()),
            None => self.globals.get(name).cloned().ok_or_else(|| invalid(occurrence)),
        }
    }
}

fn definition(expected: &ExpectedCheckedPointerCast, point: CfgPointId) -> Result<ReferenceOrigin, ReferenceAnalysisError> {
    if expected.source == CastSource::Null { return Ok(ReferenceOrigin::Null { span: expected.occurrence.range }); }
    let ScalarExpression::Call { type_arguments, .. } = &expected.call else { return Err(invalid(&expected.occurrence)); };
    let [destination] = type_arguments.as_slice() else { return Err(invalid(&expected.occurrence)); };
    let ScalarType::CheckedReference { mutability, .. } = &destination.ty else { return Err(invalid(&expected.occurrence)); };
    let binding = match &expected.source {
        CastSource::LexicalBinding(declaration) => declaration.binding.clone(),
        CastSource::CallOutput => ReferenceBindingId { source: expected.occurrence.source.clone(), function_span: expected.function_span,
            declaration_span: expected.occurrence.range, block_span: expected.function_span, kind: ReferenceBindingKind::CallOutput { point, output: 0 } },
        CastSource::Null => return Err(invalid(&expected.occurrence)),
    };
    let place = match &expected.raw {
        ScalarExpression::Name { name, span } => ScalarPlace::Name { name: name.clone(), span: *span },
        ScalarExpression::RawAddress { place, .. } => place.clone(),
        _ => ScalarPlace::Name { name: "core.pointer_cast".to_owned(), span: expected.occurrence.range },
    };
    Ok(ReferenceOrigin::Fresh {
        allocation: ReturnedAllocationId { source: binding.source.clone(), function_span: binding.function_span,
            declaration_span: binding.declaration_span, creation_span: expected.occurrence.range },
        loan: ReferenceLoanId { mode: *mutability, origin_place: ReferencePlaceId { source: binding.source.clone(), function_span: binding.function_span,
            declaration_span: binding.declaration_span, block_span: binding.block_span, binding, projections: Vec::new(), place },
            creation_source: expected.occurrence.source.clone(), creation_span: expected.occurrence.range, parent: None },
    })
}

fn value_shape<'a>(cfg: &'a ScalarReferenceCfg, value: &EvaluatedValueId, occurrence: &SourceSpan, policy: &source_boundary::SourceCapturePolicyContext, expressions: &[ResolvedExpressionOutputFact]) -> Result<&'a ResolvedValueShape, ReferenceAnalysisError> {
    let fail = || invalid(occurrence);
    if value.resolved.producer != cfg.producer_owner || value.resolved.occurrence != *occurrence
        || value.resolved.output != 0 || !value.resolved.output_projection.is_empty() { return Err(fail()); }
    let values = cfg.evaluated_values.values.iter().filter(|row| row.id == *value).collect::<Vec<_>>();
    let [value] = values.as_slice() else { return Err(invalid(occurrence)); };
    let shapes = cfg.evaluated_values.shapes.iter().filter(|row| row.id == value.shape).collect::<Vec<_>>();
    let [shape] = shapes.as_slice() else { return Err(invalid(occurrence)); };
    let owner_shapes = cfg.evaluated_values.shapes.iter().filter(|row| row.id.producer == cfg.producer_owner).collect::<Vec<_>>();
    if shape.id.producer != cfg.producer_owner || owner_shapes.get(shape.id.index).copied() != Some(*shape) { return Err(fail()); }
    let facts = expressions.iter().filter(|row| row.source_span == *occurrence).collect::<Vec<_>>();
    let [fact] = facts.as_slice() else { return Err(fail()); };
    let output = fact.outputs.outputs.first().ok_or_else(fail)?;
    if shape.ty != output.ty { return Err(fail()); }
    reference::source_epoch::SourceEpoch::canonical_shape(&cfg.evaluated_values.shapes, &shape.id, &output.ty, policy,
        &policy.structs, &policy.enums, &mut Vec::new()).map_err(|_| fail())?;
    Ok(shape)
}

fn raw_operand_inventory<'a>(place: &'a ScalarPlace, path: &mut Vec<RawAddressOperandStep>, operands: &mut Vec<(RawAddressOperandRole, Vec<RawAddressOperandStep>, &'a ScalarExpression)>) {
    match place {
        ScalarPlace::Name { .. } => {},
        ScalarPlace::Dereference { pointer, .. } => {
            path.push(RawAddressOperandStep::Pointer);
            operands.push((RawAddressOperandRole::Pointer, path.clone(), pointer));
            path.pop();
        },
        ScalarPlace::Field { base, .. } => {
            path.push(RawAddressOperandStep::Base);
            raw_operand_inventory(base, path, operands);
            path.pop();
        },
        ScalarPlace::Index { base, index, .. } => {
            path.push(RawAddressOperandStep::Base);
            raw_operand_inventory(base, path, operands);
            path.pop();
            path.push(RawAddressOperandStep::Index);
            operands.push((RawAddressOperandRole::Index, path.clone(), index));
            path.pop();
        },
    }
}

fn declaration_type(declaration: &Declaration, assignments: &[ResolvedAssignmentTypeFact], occurrence: &SourceSpan) -> Result<ScalarType, ReferenceAnalysisError> {
    let fail = || invalid(occurrence);
    let source = identity(&declaration.original.owner);
    if declaration.binding.source != *source { return Err(fail()); }
    match &declaration.type_source {
        DeclarationTypeSource::Declared(ty) => Ok(ty.clone()),
        DeclarationTypeSource::Assignment { occurrence: assignment, ordinal } => {
            let SourceNameDeclarationSite::Assignment { occurrence: original, ordinal: original_ordinal } = &declaration.original.site else { return Err(fail()); };
            if original != assignment || original_ordinal != ordinal || assignment.source_span.source != *source { return Err(fail()); }
            let facts = assignments.iter().filter(|fact| fact.occurrence == *assignment).collect::<Vec<_>>();
            let [fact] = facts.as_slice() else { return Err(fail()); };
            let output = fact.replacements.outputs.get(*ordinal).ok_or_else(fail)?;
            Ok(output.ty.clone())
        },
    }
}

fn raw_place_type(cfg: &ScalarReferenceCfg, inventory: &Inventory, expected: &ExpectedCheckedPointerCast, place: &ScalarPlace, operands: &[evaluated_value::RawAddressOperandInput], path: &mut Vec<RawAddressOperandStep>, policy: &source_boundary::SourceCapturePolicyContext, expressions: &[ResolvedExpressionOutputFact], assignments: &[ResolvedAssignmentTypeFact]) -> Result<ScalarType, ReferenceAnalysisError> {
    let fail = || invalid(&expected.occurrence);
    match place {
        ScalarPlace::Name { .. } => {
            let CastSource::LexicalBinding(declaration) = &expected.source else { return Err(fail()); };
            declaration_type(declaration, assignments, &expected.occurrence)
        },
        ScalarPlace::Dereference { pointer, .. } => {
            path.push(RawAddressOperandStep::Pointer);
            let inputs = operands.iter().filter(|input| input.role == RawAddressOperandRole::Pointer && input.path == *path && &input.expression == pointer.as_ref()).collect::<Vec<_>>();
            path.pop();
            let [input] = inputs.as_slice() else { return Err(fail()); };
            let occurrence = SourceSpan::new(expected.occurrence.source.clone(), expression_span(pointer));
            operand(cfg, inventory, &expected.occurrence, expected.block, pointer, input.value.point)?;
            let shape = value_shape(cfg, &input.value, &occurrence, policy, expressions)?;
            match &shape.ty {
                ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => Ok(inner.as_ref().clone()),
                _ => Err(fail()),
            }
        },
        ScalarPlace::Field { base, field, .. } => {
            path.push(RawAddressOperandStep::Base);
            let base_type = raw_place_type(cfg, inventory, expected, base, operands, path, policy, expressions, assignments)?;
            path.pop();
            let structure = match base_type {
                ScalarType::Struct(structure) => structure,
                ScalarType::RawPointer(inner) => match *inner {
                    ScalarType::Struct(structure) => structure,
                    _ => return Err(fail()),
                },
                _ => return Err(fail()),
            };
            let ScalarFieldReference::Resolved(field) = field else { return Err(fail()); };
            if field.structure != structure { return Err(fail()); }
            let structures = policy.structs.iter().filter(|row| row.id == structure).collect::<Vec<_>>();
            let [structure] = structures.as_slice() else { return Err(fail()); };
            let original = structure.fields.get(field.index).ok_or_else(fail)?;
            if original.id != *field { return Err(fail()); }
            Ok(original.ty.clone())
        },
        ScalarPlace::Index { base, index, .. } => {
            path.push(RawAddressOperandStep::Base);
            let base_type = raw_place_type(cfg, inventory, expected, base, operands, path, policy, expressions, assignments)?;
            path.pop();
            path.push(RawAddressOperandStep::Index);
            let inputs = operands.iter().filter(|input| input.role == RawAddressOperandRole::Index && input.path == *path && &input.expression == index.as_ref()).collect::<Vec<_>>();
            path.pop();
            let [input] = inputs.as_slice() else { return Err(fail()); };
            let occurrence = SourceSpan::new(expected.occurrence.source.clone(), expression_span(index));
            operand(cfg, inventory, &expected.occurrence, expected.block, index, input.value.point)?;
            let shape = value_shape(cfg, &input.value, &occurrence, policy, expressions)?;
            if shape.ty != ScalarType::U64 { return Err(fail()); }
            match base_type {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => Ok(*element),
                _ => Err(fail()),
            }
        },
    }
}

fn validate_raw(cfg: &ScalarReferenceCfg, inventory: &Inventory, expected: &ExpectedCheckedPointerCast, point: CfgPointId, policy: &source_boundary::SourceCapturePolicyContext, expressions: &[ResolvedExpressionOutputFact], assignments: &[ResolvedAssignmentTypeFact]) -> Result<(), ReferenceAnalysisError> {
    let ScalarExpression::RawAddress { place, .. } = &expected.raw else { return Ok(()); };
    let fail = || invalid(&expected.occurrence);
    direct_storage::validate_fields(place, &policy.structs, &expected.occurrence)?;
    let production = evaluated_raw_address::production(cfg, point)?;
    let terminal = match (&production.source, &expected.source) {
        (RawAddressInputSource::CompilerStorage, CastSource::LexicalBinding(declaration)) => {
            let input = evaluated_raw_address::source_input(cfg, point)?;
            let mut projections = Vec::new();
            super::super::super::collect_reference_projections(place, None, &mut projections);
            let binding = &declaration.binding;
            let required = ReferencePlaceId { source: expected.occurrence.source.clone(), function_span: expected.function_span,
                declaration_span: binding.declaration_span, block_span: binding.block_span, binding: binding.clone(), projections, place: place.clone() };
            if input.place != required || input.value != production.value || input.read >= point || !connected(cfg, input.read, point) { return Err(fail()); }
            input.read
        },
        (RawAddressInputSource::ThroughPointer { pointer, place: actual }, CastSource::CallOutput) => {
            let ScalarPlace::Dereference { pointer: expression, .. } = super::super::super::reference_root(place) else { return Err(fail()); };
            if actual != place || pointer.resolved.output != 0 || !pointer.resolved.output_projection.is_empty() || !connected(cfg, pointer.point, point) { return Err(fail()); }
            operand(cfg, inventory, &expected.occurrence, expected.block, expression, pointer.point)?;
            point
        },
        _ => return Err(fail()),
    };
    let mut required = Vec::new();
    raw_operand_inventory(place, &mut Vec::new(), &mut required);
    if production.operands.len() != required.len() { return Err(fail()); }
    let mut points = Vec::new();
    for (input, (role, path, expression)) in production.operands.iter().zip(&required) {
        if input.role != *role || input.path != *path || &input.expression != *expression { return Err(fail()); }
        let occurrence = SourceSpan::new(expected.occurrence.source.clone(), expression_span(expression));
        let value = &input.value;
        if value.resolved.producer != cfg.producer_owner || value.resolved.occurrence != occurrence
            || value.resolved.output != 0 || !value.resolved.output_projection.is_empty() || value.point >= terminal
            || !connected(cfg, value.point, terminal) { return Err(fail()); }
        if *role == RawAddressOperandRole::Pointer {
            let RawAddressInputSource::ThroughPointer { pointer, .. } = &production.source else { return Err(fail()); };
            if value != pointer { return Err(fail()); }
        }
        operand(cfg, inventory, &expected.occurrence, expected.block, expression, value.point)?;
        let shape = value_shape(cfg, value, &occurrence, policy, expressions)?;
        match role {
            RawAddressOperandRole::Pointer if matches!(shape.ty, ScalarType::RawPointer(_) | ScalarType::CheckedReference { .. }) => {},
            RawAddressOperandRole::Index if shape.ty == ScalarType::U64 => {},
            _ => return Err(fail()),
        }
        points.push(value.point);
    }
    for pair in points.windows(2) {
        if pair[0] >= pair[1] || !connected(cfg, pair[0], pair[1]) { return Err(fail()); }
    }
    let place_type = raw_place_type(cfg, inventory, expected, place, &production.operands, &mut Vec::new(), policy, expressions, assignments)?;
    let occurrence = SourceSpan::new(expected.occurrence.source.clone(), expression_span(&expected.raw));
    let shape = value_shape(cfg, &production.value, &occurrence, policy, expressions)?;
    let ScalarType::RawPointer(inner) = &shape.ty else { return Err(fail()); };
    if place_type != **inner { return Err(fail()); }
    Ok(())
}

fn validate_cast_segment(cfg: &ScalarReferenceCfg, argument: CfgPointId, cast: CfgPointId, occurrence: &SourceSpan) -> Result<(), ReferenceAnalysisError> {
    let fail = || invalid(occurrence);
    let argument_point = cfg.initial_points.get(argument.0).ok_or_else(fail)?;
    let cast_point = cfg.initial_points.get(cast.0).ok_or_else(fail)?;
    if argument_point.id != argument || cast_point.id != cast || argument >= cast
        || argument_point.source_span.source != occurrence.source || cast_point.source_span.source != occurrence.source
        || argument_point.scope != cast_point.scope || argument_point.successors.as_slice() != [cast] { return Err(fail()); }
    let mut incoming = Vec::new();
    for point in &cfg.initial_points {
        for successor in &point.successors {
            if *successor == cast { incoming.push(point.id); }
        }
    }
    let [predecessor] = incoming.as_slice() else { return Err(fail()); };
    if *predecessor != argument { return Err(fail()); }
    Ok(())
}

pub(in crate::scalar) fn checked_pointer_cast_constructors(cfg: &ScalarReferenceCfg, original: &source_boundary::module_resolver_input::SourceModuleResolverInputEvidence, forest: &source_boundary::SourceCaptureConstructionForest) -> Result<Vec<SourceCheckedPointerCastConstructorRef>, ReferenceAnalysisError> {
    let fail = || invalid(&cfg.initial_points[0].source_span);
    let NameResolutionContext::Complete(context) = &cfg.name_resolution_context else { return Err(fail()); };
    if context.owner != cfg.producer_owner { return Err(fail()); }
    let policy = original.policy_context(&cfg.producer_owner).map_err(|_| fail())?;
    let expressions = original.expression_epoch(identity(&cfg.producer_owner)).map_err(|_| fail())?;
    let assignments = original.assignment_epoch(identity(&cfg.producer_owner)).map_err(|_| fail())?;
    if context.expressions.as_slice() != expressions { return Err(fail()); }
    for (index, shape) in cfg.evaluated_values.shapes.iter().filter(|row| row.id.producer == cfg.producer_owner).enumerate() {
        if shape.id.index != index || cfg.evaluated_values.shapes.iter().filter(|row| row.id == shape.id).count() != 1 { return Err(fail()); }
    }
    let active_points = source_boundary::checked_pointer_cast_constructor::original_control::validate(cfg, original, forest)?;
    let module_original = source_boundary::checked_pointer_cast_constructor::original_control::own_module_original(identity(&cfg.producer_owner), original, forest)?;
    let (_, items, _, _) = original.inventory()?;
    super::super::canonical_module_root(module_original, identity(&cfg.producer_owner), &items)?;
    match &cfg.producer_owner {
        ProducerCfgOwnerId::Module(_) => {},
        ProducerCfgOwnerId::Function { .. } => {
            source_boundary::checked_pointer_cast_constructor::original_control::own_function_original(identity(&cfg.producer_owner), &cfg.producer_owner, original, forest)?;
        },
        ProducerCfgOwnerId::Literal { .. } => {
            source_boundary::checked_pointer_cast_constructor::original_control::own_literal_chain(cfg, original, forest)?;
        },
    }
    source_boundary::checked_pointer_cast_constructor::original_control::upstream_closure(cfg, original, forest)?;
    let mut active = cfg.clone();
    active.points = active_points.to_vec();
    active.initial_points = active_points.to_vec();
    let cfg = &active;
    let inventory = expected(context, &policy.structs)?;
    let mut initial = cfg.clone();
    initial.points = cfg.initial_points.clone();
    let mut visited = Vec::new();
    let mut events = BTreeSet::new();
    let mut required = Vec::new();
    for expected in &inventory.checked_pointer_casts {
        if visited.contains(expected) { continue; }
        visited.push(expected.clone());
        let fail = || invalid(&expected.occurrence);
        let ScalarExpression::Call { occurrence, type_arguments, .. } = &expected.call else { return Err(fail()); };
        let [destination] = type_arguments.as_slice() else { return Err(fail()); };
        if occurrence.source_span != expected.occurrence || !occurrence.output_projection.is_empty() { return Err(fail()); }
        let points = cfg.initial_points.iter().filter(|point| point.source_span == expected.occurrence && scope_matches(&cfg.initial_points, point, expected.block)
            && matches!(&point.kind, CfgPointKind::Call { expression, target: ResolvedCallTarget::Core(CoreOperationId::PointerCast), output: 0, .. } if expression == &expected.call)).collect::<Vec<_>>();
        if points.len() != inventory.checked_pointer_casts.iter().filter(|row| *row == expected).count() { return Err(fail()); }
        for point in points {
            if cfg.initial_points.get(point.id.0) != Some(point) || !events.insert(point.id) { return Err(fail()); }
            let CfgPointKind::Call { argument_points, .. } = &point.kind else { return Err(fail()); };
            let [argument_point] = argument_points.as_slice() else { return Err(fail()); };
            if *argument_point >= point.id || !connected(cfg, *argument_point, point.id) { return Err(fail()); }
            operand(cfg, &inventory, &expected.occurrence, expected.block, &expected.raw, *argument_point)?;
            let inputs = cfg.evaluated_values.call_argument_inputs.iter().filter(|row| row.event == SourceEventId { owner: cfg.producer_owner.clone(), point: point.id }).collect::<Vec<_>>();
            let [input] = inputs.as_slice() else { return Err(fail()); };
            source_boundary::validate_call_arguments(&initial, input)?;
            let [argument] = input.arguments.as_slice() else { return Err(fail()); };
            let raw_occurrence = SourceSpan::new(expected.occurrence.source.clone(), expression_span(&expected.raw));
            let raw_shape = value_shape(cfg, &argument.source, &raw_occurrence, &policy, expressions)?;
            if argument.ordinal != 0 || argument.source.point != *argument_point || argument.source.resolved.producer != cfg.producer_owner
                || argument.source.resolved.occurrence != raw_occurrence || argument.source.resolved.output != 0 || !argument.source.resolved.output_projection.is_empty()
                || raw_shape.id != argument.shape || !matches!(raw_shape.ty, ScalarType::RawPointer(_)) || !matches!(raw_shape.payload, ValuePayloadShape::Raw { .. }) { return Err(fail()); }
            if matches!(expected.raw, ScalarExpression::Name { .. }) {
                if let CastSource::LexicalBinding(declaration) = &expected.source {
                    if raw_shape.ty != declaration_type(declaration, assignments, &expected.occurrence)? { return Err(fail()); }
                }
            }
            validate_raw(&initial, &inventory, expected, *argument_point, &policy, expressions, assignments)?;
            let productions = cfg.evaluated_values.productions.iter().filter(|row| row.producer == cfg.producer_owner && row.occurrence == expected.occurrence && row.output_projection == occurrence.output_projection).collect::<Vec<_>>();
            let [production] = productions.as_slice() else { return Err(fail()); };
            let [output] = production.outputs.as_slice() else { return Err(fail()); };
            let shape = value_shape(cfg, output, &expected.occurrence, &policy, expressions)?;
            let calls = cfg.call_points.iter().filter(|row| row.occurrence == **occurrence).collect::<Vec<_>>();
            let [call] = calls.as_slice() else { return Err(fail()); };
            if production.evaluation_point != point.id || output.point != point.id || output.resolved.producer != cfg.producer_owner || output.resolved.occurrence != expected.occurrence
                || output.resolved.output != 0 || output.resolved.output_projection != occurrence.output_projection || shape.ty != destination.ty
                || !matches!(shape.payload, ValuePayloadShape::Checked { .. }) || call.evaluated_point != point.id || call.projections != vec![ScalarCallOutputProjection { output: 0, point: point.id }]
                || !matches!(&production.source, OutputProductionSource::Call { occurrence: actual, projections } if actual == occurrence.as_ref() && projections == &call.projections) { return Err(fail()); }
            let definition = definition(expected, point.id)?;
            let Some(origins) = &point.possible_origins else { return Err(fail()); };
            let origins = origins.iter().copied().collect::<Vec<_>>();
            let [origin] = origins.as_slice() else { return Err(fail()); };
            if cfg.facts.get(origin.0) != Some(&definition) || cfg.facts.iter().position(|row| row == &definition) != Some(origin.0) { return Err(fail()); }
            validate_cast_segment(cfg, *argument_point, point.id, &expected.occurrence)?;
            required.push(SourceCheckedPointerCastConstructorRef { origin: source_boundary::SourceConstructorOriginRef { owner: cfg.producer_owner.clone(), origin: *origin },
                event: SourceEventId { owner: cfg.producer_owner.clone(), point: point.id }, argument: SourceEventId { owner: cfg.producer_owner.clone(), point: *argument_point } });
        }
    }
    for point in &cfg.initial_points {
        if let CfgPointKind::Call { expression: ScalarExpression::Call { type_arguments, .. }, target: ResolvedCallTarget::Core(CoreOperationId::PointerCast), output, .. } = &point.kind {
            let [destination] = type_arguments.as_slice() else { return Err(invalid(&point.source_span)); };
            if matches!(destination.ty, ScalarType::CheckedReference { .. }) && (*output != 0 || !events.contains(&point.id)) { return Err(invalid(&point.source_span)); }
            if matches!(destination.ty, ScalarType::RawPointer(_)) && events.contains(&point.id) { return Err(invalid(&point.source_span)); }
        }
    }
    required.sort_by_key(|row| row.event.point);
    Ok(required)
}

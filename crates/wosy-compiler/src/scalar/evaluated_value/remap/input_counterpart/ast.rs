use super::*;
pub(super) mod outputs;
pub(super) mod signatures;
mod constructors;

#[derive(Clone, Debug, Eq, PartialEq)]
enum OriginalAstStep {
    Capture(SourceEventId),
    CaptureInput(usize),
    Point(CfgPointId),
    Body,
    Item(usize),
    Expression(usize),
    Output(usize),
    Receiver(usize),
    Value(usize),
    Target(usize),
    Field(usize),
    Argument(usize),
    Condition,
    Then,
    Else,
    Left,
    Right,
    Operand,
    Place,
    Base,
    Index,
    AllocationLength,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OriginalAstLocation {
    owner: ProducerCfgOwnerId,
    path: Vec<OriginalAstStep>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OriginalSyntaxOutputPair {
    source_location: OriginalAstLocation,
    destination_location: OriginalAstLocation,
    source: ScalarOutputValue,
    destination: ScalarOutputValue,
}

#[cfg(test)]
pub(super) fn annotation_counts(records: &[OriginalSyntaxOutputPair]) -> (usize, usize) {
    (records.len(), records.iter().filter(|record| record.source.ty == ScalarType::Error && record.destination.ty == ScalarType::Error).count())
}

#[cfg(test)]
pub(super) fn proof_candidates(checked: &CheckedBoundaryInputTypes) -> Vec<CheckedBoundaryInputTypes> {
    let mut candidates = Vec::new();
    for (index, record) in checked.syntax_outputs.iter().enumerate() {
        for defect in 0..5 {
            let mut candidate = checked.clone();
            let actual = &mut candidate.syntax_outputs[index];
            match defect {
                0 => actual.destination.ty = if record.destination.ty == ScalarType::Error { ScalarType::Unit } else { ScalarType::Error },
                1 => actual.destination.position = usize::MAX,
                2 => actual.destination.span = ByteSpan::new(0, 0),
                3 => actual.destination_location.path.push(OriginalAstStep::Output(usize::MAX)),
                4 => actual.destination.value = ScalarExpression::Boolean { value: false, span: ByteSpan::new(0, 0) },
                _ => unreachable!(),
            }
            candidates.push(candidate);
        }
    }
    candidates.extend(outputs::proof_candidates(checked));
    candidates.extend(signatures::proof_candidates(checked));
    candidates
}

pub(super) fn roots(remap: &ValueSchemaRemap, checked: &mut CheckedBoundaryInputTypes, owners: &[ProducerCfgOwnerId]) -> Result<(), ValueSchemaRemapError> {
    for owner in owners {
        let source = carrier(&checked.source_forest, owner)?.clone();
        let qualification = checked.qualification(remap, owner)?;
        let destination = carrier(&checked.destination_forest, qualification.destination())?.clone();
        direct_addresses(remap, checked, &source.resource_construction, &destination.resource_construction)?;
        allocation_addresses(remap, checked, &source.resource_construction, &destination.resource_construction)?;
        for point in &source.resource_construction.original().initial_points {
            let CfgPointKind::Capture { literal: span, parameters, parameter_spans, body, .. } = &point.kind else { continue; };
            let event = qualification.point(remap, &SourceEventId { owner: owner.clone(), point: point.id })?;
            let targets = destination.resource_construction.original().initial_points.iter().filter(|row| row.id == event.point).collect::<Vec<_>>();
            let [target] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point: point.id }); };
            let CfgPointKind::Capture { literal: actual_span, parameters: actual_parameters, parameter_spans: actual_parameter_spans, body: actual_body, .. } = &target.kind else { return Err(ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point: point.id }); };
            literal(remap, checked, source.resource_construction.original(), destination.resource_construction.original(), parameters, parameter_spans, body, *span, actual_parameters, actual_parameter_spans, actual_body, *actual_span, &[OriginalAstStep::Capture(SourceEventId { owner: owner.clone(), point: point.id })])?;
        }
        for point in &source.resource_construction.original().initial_points {
            let CfgPointKind::AssignmentDestination { place, storage } = &point.kind else { continue; };
            let event = qualification.point(remap, &SourceEventId { owner: owner.clone(), point: point.id })?;
            let targets = destination.resource_construction.original().initial_points.iter().filter(|row| row.id == event.point).collect::<Vec<_>>();
            let [target] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point: point.id }); };
            let CfgPointKind::AssignmentDestination { place: actual_place, storage: actual_storage } = &target.kind else { return Err(ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point: point.id }); };
            let roots = source.resource_construction.original().evaluated_values.destinations.iter().filter(|row| row.point == point.id).collect::<Vec<_>>();
            let [root] = roots.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point: point.id }); };
            let mut visitor = InputVisitor { remap, checked, source: source.resource_construction.original(), destination: destination.resource_construction.original(), root: root.shape.clone(), event: SourceEventId { owner: owner.clone(), point: point.id }, source_location: OriginalAstLocation { owner: owner.clone(), path: vec![OriginalAstStep::Point(point.id), OriginalAstStep::Place] }, destination_location: OriginalAstLocation { owner: destination.producer.clone(), path: vec![OriginalAstStep::Point(target.id), OriginalAstStep::Place] } };
            let actual_roots = destination.resource_construction.original().evaluated_values.destinations.iter().filter(|row| row.point == target.id).collect::<Vec<_>>();
            let [actual_root] = actual_roots.as_slice() else { return Err(visitor.invalid()); };
            visitor.require(root.storage == *storage && actual_root.storage == *actual_storage && qualification.shape(remap, &root.shape)? == actual_root.shape)?;
            match (storage, actual_storage) {
                (EvaluatedDestinationStorage::Direct(a), EvaluatedDestinationStorage::Direct(b)) => visitor.require(visitor.binding_reference(&a.binding)? == b.binding)?,
                (EvaluatedDestinationStorage::Inferred(a), EvaluatedDestinationStorage::Inferred(b)) => visitor.require(visitor.binding_reference(a)? == *b)?,
                (EvaluatedDestinationStorage::Through { pointer: a, cases: x }, EvaluatedDestinationStorage::Through { pointer: b, cases: y }) => {
                    outputs::value(&mut visitor, a, b)?;
                    visitor.require(x.len() == y.len())?;
                    for (a, b) in x.iter().zip(y) { visitor.require(a.access == b.access && visitor.binding_reference(&a.place.binding)? == b.place.binding)?; }
                }
                _ => return Err(visitor.invalid()),
            }
            visitor.place(place, actual_place)?;
            if let (EvaluatedDestinationStorage::Direct(a), EvaluatedDestinationStorage::Direct(b)) = (storage, actual_storage) {
                visitor.require(a.projections.len() == b.projections.len())?;
                for (a_projection, b_projection) in a.projections.iter().zip(&b.projections) {
                    if let (ReferencePlaceProjection::Index(a_expression), ReferencePlaceProjection::Index(b_expression)) = (a_projection, b_projection) {
                        let a_value = index_input(source.resource_construction.original(), &a.binding, &a.projections, a_expression)?;
                        let b_value = index_input(destination.resource_construction.original(), &b.binding, &b.projections, b_expression)?;
                        index(remap, visitor.checked, source.resource_construction.original(), destination.resource_construction.original(), a_expression, b_expression, &a_value, &b_value)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn literal(remap: &ValueSchemaRemap, checked: &mut CheckedBoundaryInputTypes, before: &ScalarReferenceCfg, after: &ScalarReferenceCfg, parameters: &[String], parameter_spans: &[ByteSpan], body: &ScalarBlock, span: ByteSpan, actual_parameters: &[String], actual_parameter_spans: &[ByteSpan], actual_body: &ScalarBlock, actual_span: ByteSpan, path: &[OriginalAstStep]) -> Result<(), ValueSchemaRemapError> {
    let points = before.initial_points.iter().filter(|point| matches!(&point.kind, CfgPointKind::Capture { literal, parameters: names, parameter_spans: spans, body: original, .. } if *literal == span && names == parameters && spans == parameter_spans && original == body)).collect::<Vec<_>>();
    let [point] = points.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(before.producer_owner.clone())); };
    let event = SourceEventId { owner: before.producer_owner.clone(), point: point.id };
    let actual_event = checked.qualification(remap, &before.producer_owner)?.point(remap, &event)?;
    let actual_points = after.initial_points.iter().filter(|point| point.id == actual_event.point && matches!(&point.kind, CfgPointKind::Capture { literal, parameters: names, parameter_spans: spans, body: original, .. } if *literal == actual_span && names == actual_parameters && spans == actual_parameter_spans && original == actual_body)).collect::<Vec<_>>();
    let [actual_point] = actual_points.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: event.owner.clone(), point: event.point }); };
    let children = before.literal_cfgs.iter().filter(|child| child.capture_point == point.id).collect::<Vec<_>>();
    let actual_children = after.literal_cfgs.iter().filter(|child| child.capture_point == actual_point.id).collect::<Vec<_>>();
    let ([child], [actual_child]) = (children.as_slice(), actual_children.as_slice()) else { return Err(ValueSchemaRemapError::MissingPoint { producer: event.owner.clone(), point: event.point }); };
    let source = carrier(&checked.source_forest, &child.reference_cfg.producer_owner)?.clone();
    let destination = carrier(&checked.destination_forest, &actual_child.reference_cfg.producer_owner)?.clone();
    let child_qualification = BoundaryInputQualification::creator(remap, &source.producer)?;
    if child_qualification.destination() != &destination.producer || source.resource_construction.original() != child.reference_cfg.as_ref() || destination.resource_construction.original() != actual_child.reference_cfg.as_ref() || parameters != actual_parameters || parameter_spans != actual_parameter_spans || span != actual_span {
        return Err(ValueSchemaRemapError::MissingPoint { producer: event.owner.clone(), point: event.point });
    }
    child_qualification.validate(&source, &destination)?;
    checked.retain_original(&source)?;
    checked.retain_original(&destination)?;
    if !checked.qualifications.contains(&child_qualification) {
        types::roots(remap, checked, &source, &destination, &child_qualification)?;
        checked.qualifications.push(child_qualification);
    }
    let ports = source.tables.boundary_signatures.iter().flat_map(|signature| &signature.ports).filter(|port| port.role == BoundaryPortRole::Output(0)).collect::<Vec<_>>();
    let [port] = ports.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: event.owner.clone(), point: event.point }); };
    let mut source_path = path.to_vec(); source_path.push(OriginalAstStep::Body);
    let mut destination_path = Vec::new();
    for step in path {
        destination_path.push(match step { OriginalAstStep::Capture(event) => OriginalAstStep::Capture(checked.qualification(remap, &event.owner)?.point(remap, event)?), step => step.clone() });
    }
    destination_path.push(OriginalAstStep::Body);
    let mut visitor = InputVisitor { remap, checked, source: source.resource_construction.original(), destination: destination.resource_construction.original(), root: port.shape.clone(), event,
        source_location: OriginalAstLocation { owner: source.producer.clone(), path: source_path }, destination_location: OriginalAstLocation { owner: destination.producer.clone(), path: destination_path } };
    outputs::literal(&mut visitor, point, actual_point, body, actual_body)?;
    let (CfgPointKind::Capture { captures: original, .. }, CfgPointKind::Capture { captures: actual, .. }) = (&point.kind, &actual_point.kind) else { return Err(visitor.invalid()); };
    visitor.require(original.len() == actual.len())?;
    for (index, (a, b)) in original.iter().zip(actual).enumerate() {
        visitor.enter(OriginalAstStep::CaptureInput(index));
        visitor.ty(&a.ty, &b.ty)?;
        visitor.ty(&a.place_type, &b.place_type)?;
        visitor.enter(OriginalAstStep::Place); visitor.place(&a.place, &b.place)?; visitor.leave();
        visitor.leave();
    }
    visitor.block(body, actual_body)
}

struct InputVisitor<'a> {
    remap: &'a ValueSchemaRemap,
    checked: &'a mut CheckedBoundaryInputTypes,
    source: &'a ScalarReferenceCfg,
    destination: &'a ScalarReferenceCfg,
    root: ValueShapeId,
    event: SourceEventId,
    source_location: OriginalAstLocation,
    destination_location: OriginalAstLocation,
}

impl InputVisitor<'_> {
    fn invalid(&self) -> ValueSchemaRemapError { ValueSchemaRemapError::MissingPoint { producer: self.event.owner.clone(), point: self.event.point } }

    fn require(&self, valid: bool) -> Result<(), ValueSchemaRemapError> {
        if valid { Ok(()) } else { Err(self.invalid()) }
    }

    fn point(&self, point: CfgPointId) -> Result<CfgPointId, ValueSchemaRemapError> {
        Ok(self.checked.qualification(self.remap, &self.source.producer_owner)?.point(self.remap, &SourceEventId { owner: self.source.producer_owner.clone(), point })?.point)
    }

    fn span(&self, source: &SourceSpan, destination: &SourceSpan) -> bool {
        source.source == *lexical(&self.source.producer_owner) && destination.source == *lexical(&self.destination.producer_owner) && source.range == destination.range
    }

    fn binding_reference(&self, source: &ReferenceBindingId) -> Result<ReferenceBindingId, ValueSchemaRemapError> {
        match self.checked.qualification(self.remap, &self.source.producer_owner)? {
            BoundaryInputQualification::Mapped { .. } => self.remap.binding(&self.source.producer_owner, source),
            BoundaryInputQualification::SharedOriginal { .. } => Ok(source.clone()),
        }
    }

    fn ty(&mut self, source: &ScalarType, destination: &ScalarType) -> Result<(), ValueSchemaRemapError> {
        *self.checked = self.checked.extend_original_types(self.remap, &self.source.producer_owner, &[source.clone()], &[destination.clone()], &self.root)?;
        self.require(self.checked.type_pair(&self.source.producer_owner, source, destination))
    }

    fn enter(&mut self, step: OriginalAstStep) {
        self.source_location.path.push(step.clone());
        self.destination_location.path.push(step);
    }

    fn leave(&mut self) {
        self.source_location.path.pop();
        self.destination_location.path.pop();
    }

    fn field(&mut self, source: &ScalarFieldReference, destination: &ScalarFieldReference) -> Result<(), ValueSchemaRemapError> {
        match (source, destination) {
            (ScalarFieldReference::Unresolved { name: a, span: x }, ScalarFieldReference::Unresolved { name: b, span: y }) => self.require(a == b && x == y),
            (ScalarFieldReference::Resolved(a), ScalarFieldReference::Resolved(b)) => {
                self.ty(&ScalarType::Struct(a.structure.clone()), &ScalarType::Struct(b.structure.clone()))?;
                self.require(self.checked.fields.iter().filter(|row| row.owner == self.source.producer_owner && row.source.id == *a && row.destination.id == *b).count() == 1)
            }
            _ => Err(self.invalid()),
        }
    }

    fn place(&mut self, source: &ScalarPlace, destination: &ScalarPlace) -> Result<(), ValueSchemaRemapError> {
        match (source, destination) {
            (ScalarPlace::Name { name: a, span: x }, ScalarPlace::Name { name: b, span: y }) => self.require(a == b && x == y),
            (ScalarPlace::Field { base: a, field: x, span: s }, ScalarPlace::Field { base: b, field: y, span: t }) => {
                self.require(s == t)?;
                self.field(x, y)?;
                self.enter(OriginalAstStep::Base); self.place(a, b)?; self.leave();
                Ok(())
            }
            (ScalarPlace::Dereference { pointer: a, span: x }, ScalarPlace::Dereference { pointer: b, span: y }) => {
                self.require(x == y)?;
                self.enter(OriginalAstStep::Operand); self.expression(a, b)?; self.leave();
                Ok(())
            }
            (ScalarPlace::Index { base: a, index: x, span: s, index_span: u }, ScalarPlace::Index { base: b, index: y, span: t, index_span: v }) => {
                self.require(s == t && u == v)?;
                self.enter(OriginalAstStep::Base); self.place(a, b)?; self.leave();
                self.enter(OriginalAstStep::Index); self.expression(x, y)?; self.leave();
                Ok(())
            }
            _ => Err(self.invalid()),
        }
    }

    fn output_sequence(&mut self, source: &ScalarOutputSequence, destination: &ScalarOutputSequence) -> Result<(), ValueSchemaRemapError> {
        self.require(source.span == destination.span && source.outputs.len() == destination.outputs.len())?;
        for (a, b) in source.outputs.iter().zip(&destination.outputs) {
            self.require(a.span == b.span)?;
            self.ty(&a.ty, &b.ty)?;
        }
        Ok(())
    }

    fn expressions(&mut self, source: &[ScalarExpression], destination: &[ScalarExpression]) -> Result<(), ValueSchemaRemapError> {
        self.require(source.len() == destination.len())?;
        for (index, (a, b)) in source.iter().zip(destination).enumerate() {
            self.enter(OriginalAstStep::Expression(index)); self.expression(a, b)?; self.leave();
        }
        Ok(())
    }

    fn expression(&mut self, source: &ScalarExpression, destination: &ScalarExpression) -> Result<(), ValueSchemaRemapError> {
        match (source, destination) {
            (ScalarExpression::Assignment(a), ScalarExpression::Assignment(b)) => self.assignment(a, b),
            (ScalarExpression::FunctionLiteral { parameters: a, parameter_spans: x, body: s, span: u }, ScalarExpression::FunctionLiteral { parameters: b, parameter_spans: y, body: t, span: v }) => {
                self.require(a == b && x == y && u == v)?;
                literal(self.remap, self.checked, self.source, self.destination, a, x, s, *u, b, y, t, *v, &self.source_location.path)
            }
            (ScalarExpression::Name { name: a, span: x }, ScalarExpression::Name { name: b, span: y }) => {
                self.require(a == b && x == y)?;
                let sources = self.source.initial_points.iter().filter(|point| point.source_span.range == *x && matches!(point.kind, CfgPointKind::Read { .. })).collect::<Vec<_>>();
                let targets = self.destination.initial_points.iter().filter(|point| point.source_span.range == *y && matches!(point.kind, CfgPointKind::Read { .. })).collect::<Vec<_>>();
                self.require(sources.len() == targets.len())?;
                for source in sources {
                    let point = self.point(source.id)?;
                    let actual = targets.iter().filter(|target| target.id == point).collect::<Vec<_>>();
                    let [actual] = actual.as_slice() else { return Err(self.invalid()); };
                    let (CfgPointKind::Read { binding: a }, CfgPointKind::Read { binding: b }) = (&source.kind, &actual.kind) else { return Err(self.invalid()); };
                    self.require(self.binding_reference(a)? == *b)?;
                }
                Ok(())
            }
            (ScalarExpression::Member { receiver: a, name: x, enum_tag: p, receiver_span: s, name_span: u, span: w }, ScalarExpression::Member { receiver: b, name: y, enum_tag: q, receiver_span: t, name_span: v, span: z }) => {
                self.require(a == b && x == y && p == q && s == t && u == v && w == z)?;
                constructors::member(self, source, destination)
            }
            (ScalarExpression::Integer { value: a, span: x }, ScalarExpression::Integer { value: b, span: y }) => self.require(a == b && x == y),
            (ScalarExpression::InvalidInteger { span: a, error_span: x }, ScalarExpression::InvalidInteger { span: b, error_span: y }) => self.require(a == b && x == y),
            (ScalarExpression::Float { value: a, spelling: x, span: s }, ScalarExpression::Float { value: b, spelling: y, span: t }) => self.require(a == b && x == y && s == t),
            (ScalarExpression::InvalidFloat { span: a }, ScalarExpression::InvalidFloat { span: b }) => self.require(a == b),
            (ScalarExpression::Boolean { value: a, span: x }, ScalarExpression::Boolean { value: b, span: y }) => self.require(a == b && x == y),
            (ScalarExpression::Char { value: a, spelling: x, span: s }, ScalarExpression::Char { value: b, spelling: y, span: t }) => self.require(a == b && x == y && s == t),
            (ScalarExpression::Utf8 { value: a, span: x }, ScalarExpression::Utf8 { value: b, span: y }) => self.require(a == b && x == y),
            (ScalarExpression::RawAddress { place: a, span: x }, ScalarExpression::RawAddress { place: b, span: y }) => {
                self.require(x == y)?; constructors::address(self, source, destination)?; self.enter(OriginalAstStep::Place); self.place(a, b)?; self.leave(); Ok(())
            }
            (ScalarExpression::CheckedAddress { mutability: a, place: x, span: s }, ScalarExpression::CheckedAddress { mutability: b, place: y, span: t }) => {
                self.require(a == b && s == t)?; constructors::address(self, source, destination)?; self.enter(OriginalAstStep::Place); self.place(x, y)?; self.leave(); Ok(())
            }
            (ScalarExpression::Dereference { place: a, span: x, occurrence: s }, ScalarExpression::Dereference { place: b, span: y, occurrence: t })
            | (ScalarExpression::IndexedRead { place: a, span: x, occurrence: s }, ScalarExpression::IndexedRead { place: b, span: y, occurrence: t }) => {
                self.require(std::mem::discriminant(source) == std::mem::discriminant(destination) && x == y && s.role == t.role && s.output_projection == t.output_projection && self.span(&s.source_span, &t.source_span))?;
                constructors::read(self, source, destination, s, t)?;
                self.enter(OriginalAstStep::Place); self.place(a, b)?; self.leave(); Ok(())
            }
            (ScalarExpression::StructLiteral { fields: a, span: x }, ScalarExpression::StructLiteral { fields: b, span: y }) => {
                self.require(x == y && a.len() == b.len())?;
                for (index, (a, b)) in a.iter().zip(b).enumerate() {
                    self.require(a.name == b.name && a.name_span == b.name_span && a.span == b.span)?;
                    self.enter(OriginalAstStep::Field(index)); self.expression(&a.value, &b.value)?; self.leave();
                }
                Ok(())
            }
            (ScalarExpression::ArrayLiteral { elements: a, span: x }, ScalarExpression::ArrayLiteral { elements: b, span: y }) => { self.require(x == y)?; self.expressions(a, b) }
            (ScalarExpression::Binary { operator: a, left: x, right: s, span: u }, ScalarExpression::Binary { operator: b, left: y, right: t, span: v }) => {
                self.require(a == b && u == v)?;
                self.enter(OriginalAstStep::Left); self.expression(x, y)?; self.leave();
                self.enter(OriginalAstStep::Right); self.expression(s, t)?; self.leave(); Ok(())
            }
            (ScalarExpression::Unary { operator: a, operand: x, span: s }, ScalarExpression::Unary { operator: b, operand: y, span: t }) => {
                self.require(a == b && s == t)?; self.enter(OriginalAstStep::Operand); self.expression(x, y)?; self.leave(); Ok(())
            }
            (ScalarExpression::Call { receiver: a, name: x, receiver_span: p, name_span: s, type_arguments: u, arguments: w, overload_selection: m, span: g, occurrence: c }, ScalarExpression::Call { receiver: b, name: y, receiver_span: q, name_span: t, type_arguments: v, arguments: z, overload_selection: n, span: h, occurrence: d }) => {
                self.require(a == b && x == y && p == q && s == t && g == h && u.len() == v.len() && w.len() == z.len()
                    && c.output_projection == d.output_projection && self.span(&c.source_span, &d.source_span))?;
                for (a, b) in u.iter().zip(v) { self.require(a.span == b.span)?; self.ty(&a.ty, &b.ty)?; }
                self.selection(m, n)?;
                signatures::call(self, source, destination, c, d)?;
                for (index, (a, b)) in w.iter().zip(z).enumerate() {
                    self.enter(OriginalAstStep::Argument(index)); self.expression(a, b)?; self.leave();
                }
                Ok(())
            }
            (ScalarExpression::If { condition: a, then_branch: x, else_branch: s, span: u }, ScalarExpression::If { condition: b, then_branch: y, else_branch: t, span: v }) => {
                self.require(u == v)?;
                self.enter(OriginalAstStep::Condition); self.expression(a, b)?; self.leave();
                self.enter(OriginalAstStep::Then); self.block(x, y)?; self.leave();
                self.enter(OriginalAstStep::Else); self.block(s, t)?; self.leave(); Ok(())
            }
            (ScalarExpression::UnitIf { condition: a, then_branch: x, span: s }, ScalarExpression::UnitIf { condition: b, then_branch: y, span: t }) => {
                self.require(s == t)?;
                self.enter(OriginalAstStep::Condition); self.expression(a, b)?; self.leave();
                self.enter(OriginalAstStep::Then); self.block(x, y)?; self.leave(); Ok(())
            }
            (ScalarExpression::Block(a), ScalarExpression::Block(b)) => self.block(a, b),
            _ => Err(self.invalid()),
        }
    }

    fn selection(&mut self, source: &Option<ScalarOverloadSelection>, destination: &Option<ScalarOverloadSelection>) -> Result<(), ValueSchemaRemapError> {
        match (source, destination) {
            (None, None) => Ok(()),
            (Some(a), Some(b)) => {
                self.require(a.arm_index == b.arm_index && a.substitutions.keys().eq(b.substitutions.keys()))?;
                for (a, b) in a.substitutions.values().zip(b.substitutions.values()) { self.ty(a, b)?; }
                Ok(())
            }
            _ => Err(self.invalid()),
        }
    }

    fn block(&mut self, source: &ScalarBlock, destination: &ScalarBlock) -> Result<(), ValueSchemaRemapError> {
        self.require(source.span == destination.span && source.unsafe_context == destination.unsafe_context
            && source.terminated_items == destination.terminated_items && source.items.len() == destination.items.len()
            && source.final_output_values.len() == destination.final_output_values.len()
            && source.occurrence.output_projection == destination.occurrence.output_projection && self.span(&source.occurrence.source_span, &destination.occurrence.source_span))?;
        outputs::block(self, source, destination)?;
        for (index, (a, b)) in source.items.iter().zip(&destination.items).enumerate() {
            self.enter(OriginalAstStep::Item(index));
            match (a, b) {
                (ScalarBlockItem::LocalBinding(a), ScalarBlockItem::LocalBinding(b)) => self.binding(a, b)?,
                (ScalarBlockItem::Expression(a), ScalarBlockItem::Expression(b)) => self.expression(a, b)?,
                (ScalarBlockItem::Assignment(a), ScalarBlockItem::Assignment(b)) => self.assignment(a, b)?,
                (ScalarBlockItem::While(a), ScalarBlockItem::While(b)) => {
                    self.require(a.span == b.span && a.occurrence.output_projection == b.occurrence.output_projection && self.span(&a.occurrence.source_span, &b.occurrence.source_span))?;
                    self.while_points(&a.occurrence, &b.occurrence)?;
                    self.enter(OriginalAstStep::Condition); self.expression(&a.condition, &b.condition)?; self.leave();
                    self.enter(OriginalAstStep::Body); self.block(&a.body, &b.body)?; self.leave();
                }
                _ => return Err(self.invalid()),
            }
            self.leave();
        }
        self.expressions(&source.expressions, &destination.expressions)?;
        for (index, (a, b)) in source.final_output_values.iter().zip(&destination.final_output_values).enumerate() {
            self.enter(OriginalAstStep::Output(index));
            self.require(a.position == b.position && a.span == b.span)?;
            match (&a.ty, &b.ty) {
                (ScalarType::Error, ScalarType::Error) => {},
                (ScalarType::Error, _) | (_, ScalarType::Error) | (ScalarType::Qualified { .. }, _) | (_, ScalarType::Qualified { .. }) => return Err(self.invalid()),
                _ => self.ty(&a.ty, &b.ty)?,
            }
            let record = OriginalSyntaxOutputPair { source_location: self.source_location.clone(), destination_location: self.destination_location.clone(), source: a.clone(), destination: b.clone() };
            if !self.checked.syntax_outputs.contains(&record) { self.checked.syntax_outputs.push(record); }
            self.expression(&a.value, &b.value)?;
            self.leave();
        }
        Ok(())
    }

    fn binding(&mut self, source: &ScalarBinding, destination: &ScalarBinding) -> Result<(), ValueSchemaRemapError> {
        self.require(source.name == destination.name && source.name_span == destination.name_span && source.span == destination.span
            && source.output_origin == destination.output_origin && source.is_allocation == destination.is_allocation
            && source.receivers.len() == destination.receivers.len() && source.output_values.len() == destination.output_values.len()
            && source.occurrence.output_projection == destination.occurrence.output_projection && self.span(&source.occurrence.source_span, &destination.occurrence.source_span))?;
        self.binding_points(&source.occurrence, &destination.occurrence)?;
        self.ty(&source.declared_type, &destination.declared_type)?;
        self.output_sequence(&source.output_sequence, &destination.output_sequence)?;
        self.enter(OriginalAstStep::Value(0)); self.expression(&source.value, &destination.value)?; self.leave();
        for (index, (a, b)) in source.receivers.iter().zip(&destination.receivers).enumerate() {
            self.require(a.name == b.name && a.name_span == b.name_span && a.span == b.span)?;
            self.ty(&a.ty, &b.ty)?;
            self.enter(OriginalAstStep::Receiver(index));
            match (&a.allocation_length, &b.allocation_length) {
                (None, None) => {},
                (Some(a), Some(b)) => { self.enter(OriginalAstStep::AllocationLength); self.expression(a, b)?; self.leave(); }
                _ => return Err(self.invalid()),
            }
            self.leave();
        }
        for (index, (a, b)) in source.output_values.iter().zip(&destination.output_values).enumerate() {
            self.require(a.position == b.position && a.span == b.span)?;
            self.ty(&a.ty, &b.ty)?;
            self.enter(OriginalAstStep::Value(index)); self.expression(&a.value, &b.value)?; self.leave();
        }
        Ok(())
    }

    fn assignment(&mut self, source: &ScalarAssignment, destination: &ScalarAssignment) -> Result<(), ValueSchemaRemapError> {
        self.require(source.span == destination.span && source.targets.len() == destination.targets.len()
            && source.occurrence.output_projection == destination.occurrence.output_projection && self.span(&source.occurrence.source_span, &destination.occurrence.source_span))?;
        self.assignment_points(&source.occurrence, &destination.occurrence)?;
        for (index, (a, b)) in source.targets.iter().zip(&destination.targets).enumerate() {
            self.require(a.receiver == b.receiver && a.receiver_span == b.receiver_span && a.target == b.target && a.target_span == b.target_span && a.span == b.span)?;
            self.enter(OriginalAstStep::Target(index)); self.place(&a.place, &b.place)?; self.leave();
        }
        self.expressions(&source.values, &destination.values)
    }

    fn binding_points(&mut self, source: &ScalarBindingOccurrence, destination: &ScalarBindingOccurrence) -> Result<(), ValueSchemaRemapError> {
        let a = self.source.binding_points.iter().filter(|row| row.occurrence == *source).collect::<Vec<_>>();
        let b = self.destination.binding_points.iter().filter(|row| row.occurrence == *destination).collect::<Vec<_>>();
        self.require(!a.is_empty() && a.len() == b.len())?;
        for a in a {
            let point = self.point(a.point)?;
            let b = b.iter().filter(|row| row.point == point && row.output == a.output).collect::<Vec<_>>();
            let [b] = b.as_slice() else { return Err(self.invalid()); };
            constructors::binding(self, a, b)?;
            match (&a.role, &b.role) {
                (ScalarBindingPointRole::FieldAssign(a), ScalarBindingPointRole::FieldAssign(b)) => self.field(&ScalarFieldReference::Resolved(a.clone()), &ScalarFieldReference::Resolved(b.clone()))?,
                (a, b) => self.require(a == b)?,
            }
        }
        Ok(())
    }

    fn assignment_points(&mut self, source: &ScalarAssignmentOccurrence, destination: &ScalarAssignmentOccurrence) -> Result<(), ValueSchemaRemapError> {
        let a = self.source.assignment_points.iter().filter(|row| row.occurrence == *source).collect::<Vec<_>>();
        let b = self.destination.assignment_points.iter().filter(|row| row.occurrence == *destination).collect::<Vec<_>>();
        self.require(!a.is_empty() && a.len() == b.len())?;
        for a in a {
            let point = self.point(a.point)?;
            let targets = b.iter().filter(|row| row.point == point && row.role == a.role && row.target == a.target).collect::<Vec<_>>();
            let [target] = targets.as_slice() else { return Err(self.invalid()); };
            constructors::assignment(self, a, target)?;
        }
        Ok(())
    }

    fn while_points(&mut self, source: &ScalarWhileOccurrence, destination: &ScalarWhileOccurrence) -> Result<(), ValueSchemaRemapError> {
        let a = self.source.while_points.iter().filter(|row| row.occurrence == *source).collect::<Vec<_>>();
        let b = self.destination.while_points.iter().filter(|row| row.occurrence == *destination).collect::<Vec<_>>();
        self.require(!a.is_empty() && a.len() == b.len())?;
        for a in a {
            let point = self.point(a.point)?;
            let targets = b.iter().filter(|row| row.point == point && row.role == a.role).collect::<Vec<_>>();
            let [target] = targets.as_slice() else { return Err(self.invalid()); };
            constructors::while_point(self, a, target)?;
        }
        Ok(())
    }
}

pub(super) fn captures(remap: &ValueSchemaRemap, checked: &CheckedBoundaryInputTypes, source: &SourceCaptureProduction, destination: &SourceCaptureProduction) -> Result<(), ValueSchemaRemapError> {
    let before = carrier(&checked.source_forest, &source.child)?;
    let after = carrier(&checked.destination_forest, &destination.child)?;
    let ports = before.tables.boundary_signatures.iter().flat_map(|signature| &signature.ports).filter(|port| port.role == BoundaryPortRole::Output(0)).collect::<Vec<_>>();
    let [port] = ports.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: source.event.owner.clone(), point: source.event.point }); };
    let mut proof = checked.clone();
    let mut visitor = InputVisitor { remap, checked: &mut proof, source: before.resource_construction.original(), destination: after.resource_construction.original(), root: port.shape.clone(), event: source.event.clone(),
        source_location: OriginalAstLocation { owner: source.child.clone(), path: vec![OriginalAstStep::Capture(source.event.clone()), OriginalAstStep::Body] },
        destination_location: OriginalAstLocation { owner: destination.child.clone(), path: vec![OriginalAstStep::Capture(destination.event.clone()), OriginalAstStep::Body] } };
    visitor.require(source.inputs.len() == destination.inputs.len())?;
    visitor.require(checked.type_pair(&source.event.owner, &source.signature, &destination.signature))?;
    for (index, (input, actual_input)) in source.inputs.iter().zip(&destination.inputs).enumerate() {
        let a = &input.capture;
        let b = &actual_input.capture;
        visitor.require(a.name == b.name && visitor.span(&a.use_span, &b.use_span) && checked.type_pair(&visitor.source.producer_owner, &a.ty, &b.ty) && checked.type_pair(&visitor.source.producer_owner, &a.place_type, &b.place_type))?;
        visitor.enter(OriginalAstStep::CaptureInput(index));
        visitor.enter(OriginalAstStep::Place); visitor.place(&a.place, &b.place)?; visitor.leave();
        visitor.leave();
        visitor.require(capture_place_type(&a.place, &a.ty, &before.policy_context, &a.name)? == &a.place_type && capture_place_type(&b.place, &b.ty, &after.policy_context, &b.name)? == &b.place_type)?;
    }
    visitor.require(*visitor.checked == *checked)
}

fn capture_place_type<'a>(place: &ScalarPlace, root: &'a ScalarType, context: &'a SourceCapturePolicyContext, name: &str) -> Result<&'a ScalarType, ValueSchemaRemapError> {
    let invalid = || ValueSchemaRemapError::MissingProducer(context.generic_scope.clone());
    match place {
        ScalarPlace::Name { name: original, .. } if original == name => Ok(root),
        ScalarPlace::Field { base, field: ScalarFieldReference::Resolved(field), .. } => {
            let ScalarType::Struct(structure) = capture_place_type(base, root, context, name)? else { return Err(invalid()); };
            let structures = context.structs.iter().filter(|row| row.id == *structure).collect::<Vec<_>>();
            let [structure] = structures.as_slice() else { return Err(invalid()); };
            let fields = structure.fields.iter().filter(|row| row.id == *field && field.structure == structure.id).collect::<Vec<_>>();
            let [field] = fields.as_slice() else { return Err(invalid()); };
            Ok(&field.ty)
        }
        ScalarPlace::Dereference { .. } => match root { ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => Ok(inner), _ => Err(invalid()) },
        ScalarPlace::Index { base, .. } => match capture_place_type(base, root, context, name)? { ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => Ok(element), _ => Err(invalid()) },
        _ => Err(invalid()),
    }
}

pub(super) fn role(remap: &ValueSchemaRemap, checked: &CheckedBoundaryInputTypes, owner: &ProducerCfgOwnerId, source: &BoundaryPortRole, transported: &BoundaryPortRole, destination: &BoundaryPortRole) -> Result<bool, ValueSchemaRemapError> {
    let (BoundaryPortRole::Effect { binding: source_binding, projection: a }, BoundaryPortRole::Effect { binding: mapped_binding, projection: m }, BoundaryPortRole::Effect { binding: actual_binding, projection: b }) = (source, transported, destination) else { return Ok(*transported == *destination); };
    if remap.binding(owner, source_binding)? != *mapped_binding || mapped_binding != actual_binding || a.len() != b.len() || m.len() != b.len() { return Ok(false); }
    let before = carrier(&checked.source_forest, owner)?;
    let actual_owner = checked.qualification(remap, owner)?.destination().clone();
    let after = carrier(&checked.destination_forest, &actual_owner)?;
    for ((a, m), b) in a.iter().zip(m).zip(b) {
        match (a, m, b) {
            (ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(a)), ReferencePlaceProjection::Field(mapped), ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(b))) => {
                if *mapped != ScalarFieldReference::Resolved(a.clone()) || checked.fields.iter().filter(|row| row.owner == *owner && row.source.id == *a && row.destination.id == *b).count() != 1 { return Ok(false); }
            }
            (ReferencePlaceProjection::Index(a), ReferencePlaceProjection::Index(m), ReferencePlaceProjection::Index(b)) => {
                if a != m { return Ok(false); }
                let a_value = index_input(before.resource_construction.original(), source_binding, match source { BoundaryPortRole::Effect { projection, .. } => projection, _ => unreachable!() }, a)?;
                let b_value = index_input(after.resource_construction.original(), actual_binding, match destination { BoundaryPortRole::Effect { projection, .. } => projection, _ => unreachable!() }, b)?;
                let mut proof = checked.clone();
                index(remap, &mut proof, before.resource_construction.original(), after.resource_construction.original(), a, b, &a_value, &b_value)?;
                if proof != *checked { return Ok(false); }
            }
            (ReferencePlaceProjection::Dereference(a), ReferencePlaceProjection::Dereference(m), ReferencePlaceProjection::Dereference(b)) => {
                if remap.checked_loan(owner, a)? != **m || m != b { return Ok(false); }
                if !before.resource_construction.original().facts.iter().any(|row| matches!(row, ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } if loan == a.as_ref())) || !after.resource_construction.original().facts.iter().any(|row| matches!(row, ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } if loan == b.as_ref())) { return Ok(false); }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

fn index_input(cfg: &ScalarReferenceCfg, binding: &ReferenceBindingId, projections: &[ReferencePlaceProjection], expression: &ScalarExpression) -> Result<EvaluatedValueId, ValueSchemaRemapError> {
    let mut inputs = Vec::new();
    for destination in &cfg.evaluated_values.destinations {
        let EvaluatedDestinationStorage::Direct(place) = &destination.storage else { continue; };
        if place.binding != *binding || place.projections != projections { continue; }
        for input in &destination.selector_inputs {
            if input.resolved.producer == cfg.producer_owner && input.resolved.occurrence.range == expression_span(expression) && !inputs.contains(input) { inputs.push(input.clone()); }
        }
    }
    let [input] = inputs.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(cfg.producer_owner.clone())); };
    Ok(input.clone())
}

fn index(remap: &ValueSchemaRemap, checked: &mut CheckedBoundaryInputTypes, source: &ScalarReferenceCfg, destination: &ScalarReferenceCfg, expression: &ScalarExpression, actual: &ScalarExpression, input: &EvaluatedValueId, actual_input: &EvaluatedValueId) -> Result<(), ValueSchemaRemapError> {
    let facts = source.evaluated_values.values.iter().filter(|fact| fact.id == *input).collect::<Vec<_>>();
    let [fact] = facts.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: source.producer_owner.clone(), point: input.point }); };
    let mut visitor = InputVisitor { remap, checked, source, destination, root: fact.shape.clone(), event: SourceEventId { owner: source.producer_owner.clone(), point: input.point },
        source_location: OriginalAstLocation { owner: source.producer_owner.clone(), path: vec![OriginalAstStep::Point(input.point), OriginalAstStep::Index] },
        destination_location: OriginalAstLocation { owner: destination.producer_owner.clone(), path: vec![OriginalAstStep::Point(actual_input.point), OriginalAstStep::Index] } };
    outputs::value(&mut visitor, input, actual_input)?;
    visitor.expression(expression, actual)
}

pub(super) fn direct_addresses(remap: &ValueSchemaRemap, checked: &mut CheckedBoundaryInputTypes, source: &BoundaryResourceConstructionEvidence, destination: &BoundaryResourceConstructionEvidence) -> Result<(), ValueSchemaRemapError> {
    let before = source.original();
    let after = destination.original();
    let source_rows = source_boundary::direct_storage_constructor::expected(before, source.structs()).map_err(|_| ValueSchemaRemapError::MissingProducer(before.producer_owner.clone()))?;
    let target_rows = source_boundary::direct_storage_constructor::expected(after, destination.structs()).map_err(|_| ValueSchemaRemapError::MissingProducer(after.producer_owner.clone()))?;
    if source_rows.len() != target_rows.len() { return Err(ValueSchemaRemapError::MissingProducer(before.producer_owner.clone())); }
    let qualification = checked.qualification(remap, &before.producer_owner)?.clone();
    for row in source_rows {
        let event = qualification.point(remap, &row.event)?;
        let targets = target_rows.iter().filter(|row| row.event == event).collect::<Vec<_>>();
        let [target] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: row.event.owner.clone(), point: row.event.point }); };
        let a = &before.initial_points[row.event.point.0];
        let b = &after.initial_points[target.event.point.0];
        let values = before.evaluated_values.values.iter().filter(|value| value.id.point == a.id).collect::<Vec<_>>();
        let [value] = values.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: row.event.owner.clone(), point: row.event.point }); };
        let mut visitor = InputVisitor { remap, checked, source: before, destination: after, root: value.shape.clone(), event: row.event.clone(),
            source_location: OriginalAstLocation { owner: before.producer_owner.clone(), path: vec![OriginalAstStep::Point(a.id), OriginalAstStep::Place] },
            destination_location: OriginalAstLocation { owner: after.producer_owner.clone(), path: vec![OriginalAstStep::Point(b.id), OriginalAstStep::Place] } };
        let (CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { place: a_place, loan: a_loan, origin: a_origin, indices: a_indices } },
            CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { place: b_place, loan: b_loan, origin: b_origin, indices: b_indices } }) = (&a.kind, &b.kind) else { return Err(visitor.invalid()); };
        visitor.require(visitor.span(&a.source_span, &b.source_span) && a.scope == b.scope && a_indices.len() == b_indices.len()
            && a_place.function_span == b_place.function_span && a_place.declaration_span == b_place.declaration_span && a_place.block_span == b_place.block_span
            && visitor.binding_reference(&a_place.binding)? == b_place.binding)?;
        match qualification {
            BoundaryInputQualification::Mapped { .. } => visitor.require(remap.origin(&before.producer_owner, *a_origin)? == *b_origin && remap.checked_loan(&before.producer_owner, a_loan)? == *b_loan)?,
            BoundaryInputQualification::SharedOriginal { .. } => visitor.require(a_origin == b_origin && a_loan == b_loan && source == destination)?,
        }
        visitor.place(&a_place.place, &b_place.place)?;
        for (a, b) in a_indices.iter().zip(b_indices) {
            visitor.require(a.projection == b.projection && visitor.point(a.value_point)? == b.value_point)?;
            outputs::point_values(&mut visitor, a.value_point, b.value_point)?;
        }
    }
    Ok(())
}

pub(super) fn allocation_addresses(remap: &ValueSchemaRemap, checked: &mut CheckedBoundaryInputTypes, source: &BoundaryResourceConstructionEvidence, destination: &BoundaryResourceConstructionEvidence) -> Result<(), ValueSchemaRemapError> {
    let before = source.original();
    let after = destination.original();
    let source_rows = source_boundary::allocation_identity_constructor::expected(before, source.structs()).map_err(|_| ValueSchemaRemapError::MissingProducer(before.producer_owner.clone()))?;
    let target_rows = source_boundary::allocation_identity_constructor::expected(after, destination.structs()).map_err(|_| ValueSchemaRemapError::MissingProducer(after.producer_owner.clone()))?;
    if source_rows.len() != target_rows.len() { return Err(ValueSchemaRemapError::MissingProducer(before.producer_owner.clone())); }
    let qualification = checked.qualification(remap, &before.producer_owner)?.clone();
    for row in source_rows {
        let event = qualification.point(remap, &row.event)?;
        let targets = target_rows.iter().filter(|row| row.event == event).collect::<Vec<_>>();
        let [target] = targets.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: row.event.owner.clone(), point: row.event.point }); };
        let a = &before.initial_points[row.event.point.0];
        let b = &after.initial_points[target.event.point.0];
        let values = before.evaluated_values.values.iter().filter(|value| value.id.point == a.id).collect::<Vec<_>>();
        let [value] = values.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: row.event.owner.clone(), point: row.event.point }); };
        let mut visitor = InputVisitor { remap, checked, source: before, destination: after, root: value.shape.clone(), event: row.event.clone(),
            source_location: OriginalAstLocation { owner: before.producer_owner.clone(), path: vec![OriginalAstStep::Point(a.id), OriginalAstStep::Place] },
            destination_location: OriginalAstLocation { owner: after.producer_owner.clone(), path: vec![OriginalAstStep::Point(b.id), OriginalAstStep::Place] } };
        let (CfgPointKind::AllocationIdentityAddress { request: x }, CfgPointKind::AllocationIdentityAddress { request: y }) = (&a.kind, &b.kind) else { return Err(visitor.invalid()); };
        visitor.require(row.association.association == target.association.association && x.association == row.association.association && y.association == target.association.association
            && visitor.span(&a.source_span, &b.source_span) && a.scope == b.scope && x.indices.len() == y.indices.len()
            && x.candidate.place.function_span == y.candidate.place.function_span && x.candidate.place.declaration_span == y.candidate.place.declaration_span
            && x.candidate.place.block_span == y.candidate.place.block_span && x.candidate.loan.mode == y.candidate.loan.mode
            && visitor.binding_reference(&x.candidate.place.binding)? == y.candidate.place.binding)?;
        match &qualification {
            BoundaryInputQualification::Mapped { .. } => visitor.require(remap.origin(&before.producer_owner, x.candidate.origin)? == y.candidate.origin
                && remap.checked_loan(&before.producer_owner, &x.candidate.loan)? == y.candidate.loan)?,
            BoundaryInputQualification::SharedOriginal { .. } => visitor.require(source == destination)?,
        }
        visitor.place(&x.candidate.place.place, &y.candidate.place.place)?;
        for (x, y) in x.indices.iter().zip(&y.indices) {
            visitor.require(x.projection == y.projection && visitor.point(x.value_point)? == y.value_point)?;
            outputs::point_values(&mut visitor, x.value_point, y.value_point)?;
        }
        let original = &before.rebind_associations[x.association.0];
        let actual = &after.rebind_associations[y.association.0];
        visitor.require(visitor.span(&original.occurrence.source_span, &actual.occurrence.source_span)
            && original.occurrence.output_projection == actual.occurrence.output_projection)?;
        for (x, y) in [(original.address_point, actual.address_point), (original.raw_point, actual.raw_point),
            (original.length_point, actual.length_point), (original.effect_point, actual.effect_point), (original.call_point, actual.call_point)] {
            visitor.require(visitor.point(x)? == y)?;
        }
        outputs::point_values(&mut visitor, original.address_point, actual.address_point)?;
        outputs::point_values(&mut visitor, original.raw_point, actual.raw_point)?;
        outputs::point_values(&mut visitor, original.length_point, actual.length_point)?;
        outputs::point_values(&mut visitor, original.call_point, actual.call_point)?;
        let (CfgPointKind::Call { expression: x, .. }, CfgPointKind::Call { expression: y, .. }) = (&before.initial_points[original.call_point.0].kind, &after.initial_points[actual.call_point.0].kind) else { return Err(visitor.invalid()); };
        visitor.expression(x, y)?;
    }
    Ok(())
}

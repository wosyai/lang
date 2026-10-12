use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedLiteralSignature {
    pub literal: SourceSpan,
    pub signature: ScalarType,
}

pub(super) fn record_signature(
    signatures: &RefCell<Vec<ResolvedLiteralSignature>>,
    source: &SourceIdentity,
    span: ByteSpan,
    signature: &ScalarType,
) {
    let literal = SourceSpan::new(source.clone(), span);
    let mut signatures = signatures.borrow_mut();
    if let Some(existing) = signatures.iter_mut().find(|entry| entry.literal == literal) {
        existing.signature = signature.clone();
    } else {
        signatures.push(ResolvedLiteralSignature {
            literal,
            signature: signature.clone(),
        });
    }
}

pub(super) fn contains_literal(body: &ScalarBlock) -> bool {
    body.items.iter().any(|item| match item {
        ScalarBlockItem::LocalBinding(binding) => {
            contains_literal_expression(&binding.value)
                || binding
                    .output_values
                    .iter()
                    .any(|output| contains_literal_expression(&output.value))
        }
        ScalarBlockItem::Expression(expression) => contains_literal_expression(expression),
        ScalarBlockItem::Assignment(assignment) => {
            assignment.values.iter().any(contains_literal_expression)
                || assignment
                    .targets
                    .iter()
                    .any(|target| contains_literal_place(&target.place))
        }
        ScalarBlockItem::While(while_) => {
            contains_literal_expression(&while_.condition) || contains_literal(&while_.body)
        }
    })
}

fn contains_literal_expression(expression: &ScalarExpression) -> bool {
    match expression {
        ScalarExpression::Assignment(assignment) => {
            assignment.values.iter().any(contains_literal_expression)
                || assignment
                    .targets
                    .iter()
                    .any(|target| contains_literal_place(&target.place))
        }
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. }
        | ScalarExpression::IndexedRead { place, .. } => contains_literal_place(place),
        ScalarExpression::FunctionLiteral { .. } => true,
        ScalarExpression::Call { arguments, .. } => {
            arguments.iter().any(contains_literal_expression)
        }
        ScalarExpression::StructLiteral { fields, .. } => fields
            .iter()
            .any(|field| contains_literal_expression(&field.value)),
        ScalarExpression::ArrayLiteral { elements, .. } => {
            elements.iter().any(contains_literal_expression)
        }
        ScalarExpression::Binary { left, right, .. } => {
            contains_literal_expression(left) || contains_literal_expression(right)
        }
        ScalarExpression::Unary { operand, .. } => contains_literal_expression(operand),
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            contains_literal_expression(condition)
                || contains_literal(then_branch)
                || contains_literal(else_branch)
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => contains_literal_expression(condition) || contains_literal(then_branch),
        ScalarExpression::Block(body) => contains_literal(body),
        _ => false,
    }
}

fn contains_literal_place(place: &ScalarPlace) -> bool {
    match place {
        ScalarPlace::Name { .. } => false,
        ScalarPlace::Field { base, .. } => contains_literal_place(base),
        ScalarPlace::Dereference { pointer, .. } => contains_literal_expression(pointer),
        ScalarPlace::Index { base, index, .. } => {
            contains_literal_place(base) || contains_literal_expression(index)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClosureCapture {
    pub name: String,
    pub binding: ReferenceBindingId,
    pub ty: ScalarType,
    pub place_type: ScalarType,
    pub use_span: SourceSpan,
    pub place: ScalarPlace,
}

struct DiscoveredCapture {
    name: String,
    binding: ReferenceBindingId,
    use_span: SourceSpan,
    place: ScalarPlace,
}

pub(super) fn captures(
    source: &SourceIdentity,
    expression: &ScalarExpression,
    outer: &BTreeMap<String, ReferenceBindingId>,
    binding_types: &HashMap<ReferenceBindingId, ScalarType>,
    structs: &[ScalarStruct],
) -> Vec<ClosureCapture> {
    let ScalarExpression::FunctionLiteral {
        parameters, body, ..
    } = expression
    else {
        panic!("capture discovery requires a function literal");
    };
    let mut local = parameters.iter().cloned().collect::<BTreeSet<_>>();
    let mut found = Vec::new();
    block(source, body, outer, &mut local, &mut found);
    found
        .into_iter()
        .map(|capture| {
            let ty = binding_types[&capture.binding].clone();
            let mut place = capture.place;
            let place_type = resolve_capture_place(&mut place, &ty, structs);
            ClosureCapture {
                name: capture.name,
                binding: capture.binding,
                ty,
                place_type,
                use_span: capture.use_span,
                place,
            }
        })
        .collect()
}

fn resolve_capture_place(
    place: &mut ScalarPlace,
    root: &ScalarType,
    structs: &[ScalarStruct],
) -> ScalarType {
    match place {
        ScalarPlace::Name { .. } => root.clone(),
        ScalarPlace::Dereference { .. } => match root {
            ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
                *inner.clone()
            }
            _ => panic!("captured dereference requires pointer binding"),
        },
        ScalarPlace::Field { base, field, .. } => {
            let base_ty = resolve_capture_place(base, root, structs);
            let ScalarType::Struct(id) = base_ty else {
                panic!("captured field requires struct binding")
            };
            let structure = structs
                .iter()
                .find(|structure| structure.id == id)
                .expect("captured struct identity");
            let declared = match field {
                ScalarFieldReference::Unresolved { name, .. } => structure
                    .fields
                    .iter()
                    .find(|candidate| candidate.name == *name)
                    .expect("validated captured field"),
                ScalarFieldReference::Resolved(field_id) => &structure.fields[field_id.index],
            };
            *field = ScalarFieldReference::Resolved(declared.id.clone());
            declared.ty.clone()
        }
        ScalarPlace::Index { base, .. } => match resolve_capture_place(base, root, structs) {
            ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                *element
            }
            _ => panic!("captured index requires array binding"),
        },
    }
}

fn name(
    source: &SourceIdentity,
    value: &str,
    span: ByteSpan,
    place: ScalarPlace,
    outer: &BTreeMap<String, ReferenceBindingId>,
    local: &BTreeSet<String>,
    found: &mut Vec<DiscoveredCapture>,
) {
    if local.contains(value) {
        return;
    }
    if let Some(binding) = outer.get(value) {
        found.push(DiscoveredCapture {
            name: value.to_owned(),
            binding: binding.clone(),
            use_span: SourceSpan::new(source.clone(), span),
            place,
        });
    }
}

fn place(
    source: &SourceIdentity,
    value: &ScalarPlace,
    outer: &BTreeMap<String, ReferenceBindingId>,
    local: &BTreeSet<String>,
    found: &mut Vec<DiscoveredCapture>,
) {
    match reference_root(value) {
        ScalarPlace::Name {
            name: value_name, ..
        } => {
            name(
                source,
                value_name,
                place_span(value),
                value.clone(),
                outer,
                local,
                found,
            );
        }
        ScalarPlace::Dereference { pointer, .. } => {
            if let ScalarExpression::Name {
                name: value_name, ..
            } = pointer.as_ref()
            {
                name(
                    source,
                    value_name,
                    place_span(value),
                    value.clone(),
                    outer,
                    local,
                    found,
                );
            } else {
                expression(source, pointer, outer, local, found);
            }
        }
        ScalarPlace::Field { .. } | ScalarPlace::Index { .. } => {
            panic!("reference root must be a name or dereference");
        }
    }
    place_indices(source, value, outer, local, found);
}

fn reference_root(place: &ScalarPlace) -> &ScalarPlace {
    match place {
        ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => place,
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => reference_root(base),
    }
}

fn place_span(value: &ScalarPlace) -> ByteSpan {
    match value {
        ScalarPlace::Name { span, .. }
        | ScalarPlace::Field { span, .. }
        | ScalarPlace::Index { span, .. }
        | ScalarPlace::Dereference { span, .. } => *span,
    }
}

fn place_indices(
    source: &SourceIdentity,
    value: &ScalarPlace,
    outer: &BTreeMap<String, ReferenceBindingId>,
    local: &BTreeSet<String>,
    found: &mut Vec<DiscoveredCapture>,
) {
    match value {
        ScalarPlace::Index { base, index, .. } => {
            place_indices(source, base, outer, local, found);
            expression(source, index, outer, local, found);
        }
        ScalarPlace::Field { base, .. } => place_indices(source, base, outer, local, found),
        ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => {}
    }
}

fn expression(
    source: &SourceIdentity,
    value: &ScalarExpression,
    outer: &BTreeMap<String, ReferenceBindingId>,
    local: &BTreeSet<String>,
    found: &mut Vec<DiscoveredCapture>,
) {
    match value {
        ScalarExpression::Assignment(assignment) => {
            for value in &assignment.values {
                expression(source, value, outer, local, found);
            }
            for target in &assignment.targets {
                place(source, &target.place, outer, local, found);
            }
        }
        ScalarExpression::FunctionLiteral {
            parameters, body, ..
        } => {
            let mut nested = local.clone();
            nested.extend(parameters.iter().cloned());
            block(source, body, outer, &mut nested, found);
        }
        ScalarExpression::Name { name: value, span } => {
            name(
                source,
                value,
                *span,
                ScalarPlace::Name {
                    name: value.clone(),
                    span: *span,
                },
                outer,
                local,
                found,
            );
        }
        ScalarExpression::Member {
            receiver,
            receiver_span,
            name: field,
            name_span,
            span,
            ..
        } => name(
            source,
            receiver,
            *span,
            ScalarPlace::Field {
                base: Box::new(ScalarPlace::Name {
                    name: receiver.clone(),
                    span: *receiver_span,
                }),
                field: ScalarFieldReference::Unresolved {
                    name: field.clone(),
                    span: *name_span,
                },
                span: *span,
            },
            outer,
            local,
            found,
        ),
        ScalarExpression::Call {
            receiver,
            receiver_span,
            name: callee,
            name_span,
            span,
            arguments,
            ..
        } => {
            if let (Some(receiver), Some(receiver_span)) = (receiver, receiver_span) {
                name(
                    source,
                    receiver,
                    *span,
                    ScalarPlace::Field {
                        base: Box::new(ScalarPlace::Name {
                            name: receiver.clone(),
                            span: *receiver_span,
                        }),
                        field: ScalarFieldReference::Unresolved {
                            name: callee.clone(),
                            span: *name_span,
                        },
                        span: *span,
                    },
                    outer,
                    local,
                    found,
                );
            } else {
                name(
                    source,
                    callee,
                    *name_span,
                    ScalarPlace::Name {
                        name: callee.clone(),
                        span: *name_span,
                    },
                    outer,
                    local,
                    found,
                );
            }
            for argument in arguments {
                expression(source, argument, outer, local, found);
            }
        }
        ScalarExpression::RawAddress { place: value, .. }
        | ScalarExpression::CheckedAddress { place: value, .. }
        | ScalarExpression::Dereference { place: value, .. }
        | ScalarExpression::IndexedRead { place: value, .. } => {
            place(source, value, outer, local, found);
        }
        ScalarExpression::Binary { left, right, .. } => {
            expression(source, left, outer, local, found);
            expression(source, right, outer, local, found);
        }
        ScalarExpression::Unary { operand, .. } => {
            expression(source, operand, outer, local, found);
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                expression(source, element, outer, local, found);
            }
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                expression(source, &field.value, outer, local, found);
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            expression(source, condition, outer, local, found);
            block(source, then_branch, outer, &mut local.clone(), found);
            block(source, else_branch, outer, &mut local.clone(), found);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            expression(source, condition, outer, local, found);
            block(source, then_branch, outer, &mut local.clone(), found);
        }
        ScalarExpression::Block(body) => {
            block(source, body, outer, &mut local.clone(), found);
        }
        _ => {}
    }
}

fn block(
    source: &SourceIdentity,
    body: &ScalarBlock,
    outer: &BTreeMap<String, ReferenceBindingId>,
    local: &mut BTreeSet<String>,
    found: &mut Vec<DiscoveredCapture>,
) {
    for item in &body.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                expression(source, &binding.value, outer, local, found);
                if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions {
                    for value in binding.output_values.iter().skip(1) {
                        expression(source, &value.value, outer, local, found);
                    }
                }
                for receiver in &binding.receivers {
                    local.insert(receiver.name.clone());
                }
            }
            ScalarBlockItem::Expression(value) => {
                expression(source, value, outer, local, found);
            }
            ScalarBlockItem::Assignment(assignment) => {
                for value in &assignment.values {
                    expression(source, value, outer, local, found);
                }
                for target in &assignment.targets {
                    place(source, &target.place, outer, local, found);
                }
            }
            ScalarBlockItem::While(while_) => {
                expression(source, &while_.condition, outer, local, found);
                block(source, &while_.body, outer, &mut local.clone(), found);
            }
        }
    }
}

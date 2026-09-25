use super::*;

pub(super) fn record_program_reference_origins(
    program: &mut ScalarProgram,
    call_sites: &[ResolvedCallSite],
) -> Vec<ReferenceAnalysisError> {
    let mut errors = record_module_reference_origins(
        &program.source,
        &mut program.items,
        call_sites,
        &program.structs,
        &program.enums,
        &program.resolved_assignment_outputs.borrow(),
    );
    if errors.is_empty() {
        let mut modules = vec![ScalarModule::from_program(program.clone(), Vec::new())];
        errors.extend(resolve_reference_output_contracts(&mut modules));
        program.items = modules.remove(0).items;
    }
    errors
}

pub(super) struct ReferenceFlowBuilder {
    pub(super) source: SourceIdentity,
    pub(super) function_span: ByteSpan,
    pub(super) names: BTreeMap<String, ReferenceBindingId>,
    pub(super) module_names: HashSet<String>,
    pub(super) call_sites: Vec<ResolvedCallSite>,
    pub(super) origins: HashMap<ReferenceBindingId, BTreeSet<ReferenceOriginId>>,
    pub(super) checked: HashSet<ReferenceBindingId>,
    pub(super) binding_types: HashMap<ReferenceBindingId, ScalarType>,
    pub(super) structs: Vec<ScalarStruct>,
    pub(super) enums: Vec<ScalarEnum>,
    pub(super) generic_parameters: BTreeSet<String>,
    pub(super) assignment_outputs: HashMap<ResolvedAssignmentOutputId, ScalarType>,
    pub(super) dereference_points: HashMap<ByteSpan, CfgPointId>,
    pub(super) cfg: ScalarReferenceCfg,
    pub(super) next_scope: usize,
    pub(super) cursor: Option<CfgPointId>,
}

#[derive(Debug)]
pub(super) enum ReferenceValue {
    Other,
    Checked(BTreeSet<ReferenceOriginId>),
    Uncomputed,
}

impl ReferenceFlowBuilder {
    fn assignment_output_types(
        &self,
        assignment: &ScalarAssignment,
    ) -> Result<Vec<ScalarType>, ReferenceAnalysisError> {
        assignment
            .targets
            .iter()
            .enumerate()
            .map(|(output_index, target)| {
                let key = ResolvedAssignmentOutputId {
                    source: self.source.clone(),
                    function_span: self.function_span,
                    target_span: target.target_span,
                    output_index,
                };
                self.assignment_outputs.get(&key).cloned().ok_or_else(|| {
                    self.missing_context(
                        target.target_span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    )
                })
            })
            .collect()
    }

    fn call_target(
        &self,
        expression: &ScalarExpression,
    ) -> Result<ResolvedCallTarget, ReferenceAnalysisError> {
        let span = expression_span(expression);
        self.call_sites
            .iter()
            .find(|site| site.source_span == SourceSpan::new(self.source.clone(), span))
            .map(|site| site.target.clone())
            .ok_or_else(|| {
                self.missing_context(
                    span,
                    ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                )
            })
    }

    fn intern(&mut self, origin: ReferenceOrigin) -> ReferenceOriginId {
        if let Some(index) = self.cfg.facts.iter().position(|fact| fact == &origin) {
            ReferenceOriginId(index)
        } else {
            let id = ReferenceOriginId(self.cfg.facts.len());
            self.cfg.facts.push(origin);
            id
        }
    }

    fn point(
        &mut self,
        kind: CfgPointKind,
        span: ByteSpan,
        scope: ReferenceScopeId,
        possible_origins: BTreeSet<ReferenceOriginId>,
    ) -> CfgPointId {
        self.point_with_origins(kind, span, scope, Some(possible_origins))
    }

    fn point_with_origins(
        &mut self,
        kind: CfgPointKind,
        span: ByteSpan,
        scope: ReferenceScopeId,
        possible_origins: Option<BTreeSet<ReferenceOriginId>>,
    ) -> CfgPointId {
        let id = CfgPointId(self.cfg.points.len());
        if let Some(previous) = self.cursor {
            self.cfg.points[previous.0].successors.push(id);
        }
        self.cfg.points.push(ReferenceFlowPoint {
            id,
            kind,
            source_span: SourceSpan::new(self.source.clone(), span),
            scope,
            possible_origins,
            successors: Vec::new(),
        });
        self.cursor = Some(id);
        id
    }

    fn uncomputed_point(&mut self, kind: CfgPointKind, span: ByteSpan, scope: ReferenceScopeId) {
        self.point_with_origins(kind, span, scope, None);
    }

    fn missing_origin(
        &self,
        span: ByteSpan,
        binding_id: ReferenceBindingId,
        reason: ReferenceAnalysisErrorReason,
    ) -> ReferenceAnalysisError {
        ReferenceAnalysisError::MissingOrigin {
            source_span: SourceSpan::new(self.source.clone(), span),
            binding_id,
            reason,
        }
    }

    fn missing_context(
        &self,
        span: ByteSpan,
        reason: ReferenceAnalysisErrorReason,
    ) -> ReferenceAnalysisError {
        ReferenceAnalysisError::MissingContext {
            source_span: SourceSpan::new(self.source.clone(), span),
            reason,
        }
    }

    fn binding(
        &self,
        name: &str,
        span: ByteSpan,
    ) -> Result<ReferenceBindingId, ReferenceAnalysisError> {
        self.names
            .get(name)
            .cloned()
            .ok_or_else(|| ReferenceAnalysisError::MissingBinding {
                source_span: SourceSpan::new(self.source.clone(), span),
                name: name.to_owned(),
            })
    }

    fn assignment_place(
        &self,
        place: &ScalarPlace,
        target_span: ByteSpan,
    ) -> Result<ReferencePlaceId, ReferenceAnalysisError> {
        let root = reference_root(place);
        let (binding, parent) = match root {
            ScalarPlace::Name { name, span } => (self.binding(name, *span)?, None),
            ScalarPlace::Dereference { pointer, .. } => {
                let ScalarExpression::Name { name, span } = pointer.as_ref() else {
                    return Err(self.missing_context(
                        target_span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                let pointer_binding = self.binding(name, *span)?;
                let origins = self.required(&pointer_binding, *span)?;
                let loans = origins
                    .iter()
                    .map(|origin| match &self.cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => Ok(loan),
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => Err(self
                            .missing_context(
                                *span,
                                ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let [loan] = loans.as_slice() else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                (loan.origin_place.binding.clone(), Some(*loan))
            }
            ScalarPlace::Field { .. } | ScalarPlace::Index { .. } => unreachable!(),
        };
        let mut projections = Vec::new();
        collect_reference_projections(place, parent, &mut projections);
        Ok(ReferencePlaceId {
            source: self.source.clone(),
            function_span: self.function_span,
            declaration_span: binding.declaration_span,
            block_span: binding.block_span,
            binding,
            projections,
            place: place.clone(),
        })
    }

    fn required(
        &self,
        id: &ReferenceBindingId,
        span: ByteSpan,
    ) -> Result<BTreeSet<ReferenceOriginId>, ReferenceAnalysisError> {
        self.origins
            .get(id)
            .filter(|set| !set.is_empty())
            .cloned()
            .ok_or_else(|| {
                self.missing_origin(
                    span,
                    id.clone(),
                    ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                )
            })
    }

    fn pointer_cast_origins(
        &mut self,
        raw: &ScalarExpression,
        mutability: ScalarReferenceMutability,
        span: ByteSpan,
    ) -> Result<BTreeSet<ReferenceOriginId>, ReferenceAnalysisError> {
        let fact = if matches!(raw, ScalarExpression::Name { name, .. } if name == "null") {
            ReferenceOrigin::Null { span }
        } else {
            let binding = match raw {
                ScalarExpression::Name {
                    name,
                    span: raw_span,
                } => self.binding(name, *raw_span)?,
                ScalarExpression::RawAddress { place, .. }
                    if matches!(reference_root(place), ScalarPlace::Name { .. }) =>
                {
                    let ScalarPlace::Name {
                        name,
                        span: raw_span,
                    } = reference_root(place)
                    else {
                        unreachable!()
                    };
                    self.binding(name, *raw_span)?
                }
                _ => ReferenceBindingId {
                    source: self.source.clone(),
                    function_span: self.function_span,
                    declaration_span: span,
                    block_span: self.function_span,
                    kind: ReferenceBindingKind::CallOutput {
                        point: CfgPointId(self.cfg.points.len()),
                        output: 0,
                    },
                },
            };
            let place = match raw {
                ScalarExpression::Name {
                    name,
                    span: raw_span,
                } => ScalarPlace::Name {
                    name: name.clone(),
                    span: *raw_span,
                },
                ScalarExpression::RawAddress { place, .. } => place.clone(),
                _ => ScalarPlace::Name {
                    name: "core.pointer_cast".to_owned(),
                    span,
                },
            };
            ReferenceOrigin::Fresh {
                allocation: ReturnedAllocationId {
                    source: binding.source.clone(),
                    function_span: binding.function_span,
                    declaration_span: binding.declaration_span,
                    creation_span: span,
                },
                loan: ReferenceLoanId {
                    mode: mutability,
                    origin_place: ReferencePlaceId {
                        source: binding.source.clone(),
                        function_span: binding.function_span,
                        declaration_span: binding.declaration_span,
                        block_span: binding.block_span,
                        binding,
                        projections: Vec::new(),
                        place,
                    },
                    creation_span: span,
                    parent: None,
                },
            }
        };
        Ok(BTreeSet::from([self.intern(fact)]))
    }

    fn conditional(
        &mut self,
        condition: &ScalarExpression,
        then_branch: &ScalarBlock,
        else_branch: Option<&ScalarBlock>,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        self.expression(condition, scope)?;
        let branch = self.point(
            CfgPointKind::Branch {
                condition: condition.clone(),
            },
            expression_span(condition),
            scope,
            BTreeSet::new(),
        );
        let incoming_names = self.names.clone();
        let incoming_origins = self.origins.clone();
        let incoming_checked = self.checked.clone();
        let then_value = self.block(then_branch)?;
        let then_end = self.cursor.expect("then branch terminal");
        let then_checked = self.checked.clone();

        self.names = incoming_names;
        self.origins = incoming_origins.clone();
        self.checked = incoming_checked.clone();
        self.cursor = Some(branch);
        let (else_value, else_end) = if let Some(else_branch) = else_branch {
            let result = self.block(else_branch)?;
            (result, self.cursor.expect("else branch terminal"))
        } else {
            (None, branch)
        };
        self.checked.extend(then_checked);
        self.origins.clear();
        self.cursor = None;
        let joined_value = then_value.is_some() && else_value.is_some();
        let join = if joined_value {
            self.point_with_origins(CfgPointKind::Join, span, scope, None)
        } else {
            self.point(CfgPointKind::Join, span, scope, BTreeSet::new())
        };
        self.cfg.points[then_end.0].successors.push(join);
        if else_end != branch {
            self.cfg.points[else_end.0].successors.push(join);
        } else {
            self.cfg.points[branch.0].successors.push(join);
        }
        Ok(if joined_value {
            ReferenceValue::Uncomputed
        } else {
            ReferenceValue::Other
        })
    }

    pub(super) fn expression(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let span = expression_span(expression);
        match expression {
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => self.conditional(condition, then_branch, Some(else_branch), span, scope),
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => self.conditional(condition, then_branch, None, span, scope),
            ScalarExpression::Block(block) => match self.block(block)? {
                Some(ReferenceValue::Checked(origins)) => {
                    self.point(
                        CfgPointKind::Evaluate(expression.clone()),
                        span,
                        scope,
                        origins.clone(),
                    );
                    Ok(ReferenceValue::Checked(origins))
                }
                Some(ReferenceValue::Uncomputed) => {
                    self.uncomputed_point(CfgPointKind::Evaluate(expression.clone()), span, scope);
                    Ok(ReferenceValue::Uncomputed)
                }
                _ => {
                    self.point(
                        CfgPointKind::Evaluate(expression.clone()),
                        span,
                        scope,
                        BTreeSet::new(),
                    );
                    Ok(ReferenceValue::Other)
                }
            },
            ScalarExpression::Call { arguments, .. } => {
                let target = self.call_target(expression)?;
                let mut address = None;
                let mut checked_release_address = None;
                let mut argument_points = Vec::new();
                for (position, argument) in arguments.iter().enumerate() {
                    self.expression(argument, scope)?;
                    argument_points.push(self.cursor.expect("evaluated call argument"));
                    if position == 0
                        && target == ResolvedCallTarget::Core(CoreOperationId::Free)
                        && (matches!(argument, ScalarExpression::CheckedAddress { .. })
                            || matches!(argument, ScalarExpression::Name { name, .. } if self.names.get(name).is_some_and(|binding| self.checked.contains(binding))))
                    {
                        checked_release_address = self.cursor;
                    }
                    if position == 0
                        && matches!(
                            target,
                            ResolvedCallTarget::Core(
                                CoreOperationId::Invalidate | CoreOperationId::Rebind
                            )
                        )
                    {
                        address = self.cursor;
                    }
                    self.move_value(argument, scope)?;
                }
                if let ResolvedCallTarget::Core(
                    operation @ (CoreOperationId::Invalidate | CoreOperationId::Rebind),
                ) = &target
                {
                    let address_point = address.expect("validated core address argument point");
                    let inputs = match operation {
                        CoreOperationId::Invalidate => InvalidationInputs::Invalidate,
                        CoreOperationId::Rebind => InvalidationInputs::Rebind {
                            raw_address: arguments[1].clone(),
                            raw_span: SourceSpan::new(
                                self.source.clone(),
                                expression_span(&arguments[1]),
                            ),
                            length: arguments[2].clone(),
                            length_span: SourceSpan::new(
                                self.source.clone(),
                                expression_span(&arguments[2]),
                            ),
                        },
                        _ => unreachable!("invalidation operation"),
                    };
                    self.point(
                        CfgPointKind::Invalidate {
                            address_point,
                            candidates: Vec::new(),
                            operation: *operation,
                            address_span: SourceSpan::new(
                                self.source.clone(),
                                expression_span(&arguments[0]),
                            ),
                            inputs,
                        },
                        span,
                        scope,
                        BTreeSet::new(),
                    );
                }
                if target == ResolvedCallTarget::Core(CoreOperationId::Free) {
                    if let Some(address_point) = checked_release_address {
                        self.point(
                            CfgPointKind::Release {
                                target: ReleaseTarget::Checked {
                                    address_point,
                                    candidates: Vec::new(),
                                    address_span: SourceSpan::new(
                                        self.source.clone(),
                                        expression_span(&arguments[0]),
                                    ),
                                },
                            },
                            span,
                            scope,
                            BTreeSet::new(),
                        );
                    } else if let [ScalarExpression::Name {
                        name: place_name,
                        span: argument_span,
                    }] = arguments.as_slice()
                    {
                        let binding = self.binding(place_name, *argument_span)?;
                        self.point(
                            CfgPointKind::Release {
                                target: ReleaseTarget::Raw { binding },
                            },
                            span,
                            scope,
                            BTreeSet::new(),
                        );
                    }
                }
                let cast_origins = match (&target, expression) {
                    (
                        ResolvedCallTarget::Core(CoreOperationId::PointerCast),
                        ScalarExpression::Call { type_arguments, .. },
                    ) => match type_arguments.as_slice() {
                        [argument] => match &argument.ty {
                            ScalarType::CheckedReference { mutability, .. } => {
                                Some(self.pointer_cast_origins(&arguments[0], *mutability, span)?)
                            }
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => None,
                };
                let call = CfgPointKind::Call {
                    expression: expression.clone(),
                    output: 0,
                    target,
                    argument_points,
                };
                if let Some(origins) = cast_origins {
                    self.point(call, span, scope, origins.clone());
                    Ok(ReferenceValue::Checked(origins))
                } else {
                    self.uncomputed_point(call, span, scope);
                    Ok(ReferenceValue::Uncomputed)
                }
            }
            ScalarExpression::Binary { left, right, .. } => {
                self.expression(left, scope)?;
                self.expression(right, scope)?;
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Unary { operand, .. } => {
                self.expression(operand, scope)?;
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for element in elements {
                    self.expression(element, scope)?;
                    self.move_value(element, scope)?;
                }
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::StructLiteral { fields, .. } => {
                for field in fields {
                    self.expression(&field.value, scope)?;
                    self.move_value(&field.value, scope)?;
                }
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Dereference { place, .. }
            | ScalarExpression::IndexedRead { place, .. } => {
                self.checked_place_read(place, expression, span, scope)
            }
            ScalarExpression::RawAddress { place, .. } => {
                self.place_expressions(place, scope)?;
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Member {
                receiver,
                name,
                receiver_span,
                ..
            } if self
                .names
                .get(receiver)
                .and_then(|binding| self.binding_types.get(binding))
                .is_some_and(|ty| matches!(ty, ScalarType::Struct(_))) =>
            {
                let binding = self.binding(receiver, *receiver_span)?;
                let ScalarType::Struct(id) = self.binding_types.get(&binding).ok_or_else(|| {
                    self.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
                })?
                else {
                    unreachable!()
                };
                let structure = self
                    .structs
                    .iter()
                    .find(|structure| structure.id == *id)
                    .ok_or_else(|| {
                        self.missing_context(
                            span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                let field = structure
                    .fields
                    .iter()
                    .find(|field| field.name == *name)
                    .ok_or_else(|| {
                        self.missing_context(
                            span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                let place = ScalarPlace::Field {
                    base: Box::new(ScalarPlace::Name {
                        name: receiver.clone(),
                        span: *receiver_span,
                    }),
                    field: ScalarFieldReference::Resolved(field.id.clone()),
                    span,
                };
                self.checked_place_read(&place, expression, span, scope)
            }
            ScalarExpression::Name { name, .. } if name == "null" => {
                let id = self.intern(ReferenceOrigin::Null { span });
                let origins = BTreeSet::from([id]);
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    origins.clone(),
                );
                Ok(ReferenceValue::Checked(origins))
            }
            ScalarExpression::Name { name, .. } => {
                if self.module_names.contains(name) && !self.names.contains_key(name) {
                    self.point(
                        CfgPointKind::Evaluate(expression.clone()),
                        span,
                        scope,
                        BTreeSet::new(),
                    );
                    return Ok(ReferenceValue::Other);
                }
                let binding = self.binding(name, span)?;
                if self.checked.contains(&binding) {
                    if self.origins.contains_key(&binding) {
                        let origins = self.required(&binding, span)?;
                        self.point(CfgPointKind::Read { binding }, span, scope, origins.clone());
                        Ok(ReferenceValue::Checked(origins))
                    } else {
                        self.uncomputed_point(CfgPointKind::Read { binding }, span, scope);
                        Ok(ReferenceValue::Uncomputed)
                    }
                } else {
                    self.point(CfgPointKind::Read { binding }, span, scope, BTreeSet::new());
                    Ok(ReferenceValue::Other)
                }
            }
            ScalarExpression::CheckedAddress {
                mutability, place, ..
            } => {
                let root = reference_root(place);
                let parents = match root {
                    ScalarPlace::Dereference { pointer, .. } => {
                        let ReferenceValue::Checked(origins) = self.expression(pointer, scope)?
                        else {
                            let binding = match pointer.as_ref() {
                                ScalarExpression::Name { name, span } => {
                                    self.binding(name, *span)?
                                }
                                _ => return Err(self.missing_context(
                                    span,
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                )),
                            };
                            return Err(self.missing_origin(
                                span,
                                binding,
                                ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            ));
                        };
                        Some(origins)
                    }
                    _ => None,
                };
                self.place_indices(place, scope)?;
                let parent_ids = match &parents {
                    Some(ids) => ids.iter().copied().map(Some).collect::<Vec<_>>(),
                    None => vec![None],
                };
                let mut origins = BTreeSet::new();
                for parent_id in parent_ids {
                    let parent = parent_id.map(|id| self.cfg.facts[id.0].clone());
                    if let Some(
                        ReferenceOrigin::Null { span: origin_span }
                        | ReferenceOrigin::Invalid { origin_span, .. },
                    ) = &parent
                    {
                        let id = self.intern(ReferenceOrigin::Invalid {
                            origin_span: *origin_span,
                            conflict_span: span,
                        });
                        origins.insert(id);
                        self.point(
                            CfgPointKind::Evaluate(expression.clone()),
                            span,
                            scope,
                            BTreeSet::from([id]),
                        );
                        continue;
                    }
                    let (binding, declaration_span, declaration_block, parent_loan) =
                        match (root, &parent) {
                            (
                                ScalarPlace::Name {
                                    name,
                                    span: name_span,
                                },
                                _,
                            ) => {
                                let binding = self.binding(name, *name_span)?;
                                (
                                    binding.clone(),
                                    binding.declaration_span,
                                    binding.block_span,
                                    None,
                                )
                            }
                            (
                                ScalarPlace::Dereference { .. },
                                Some(
                                    ReferenceOrigin::Fresh { loan, .. }
                                    | ReferenceOrigin::BorrowedFrom { loan, .. },
                                ),
                            ) => (
                                loan.origin_place.binding.clone(),
                                loan.origin_place.declaration_span,
                                loan.origin_place.block_span,
                                Some(loan.clone()),
                            ),
                            _ => {
                                return Err(self.missing_context(
                                    span,
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                ));
                            }
                        };
                    let mut projections = Vec::new();
                    collect_reference_projections(place, parent_loan.as_ref(), &mut projections);
                    let loan = ReferenceLoanId {
                        mode: *mutability,
                        origin_place: ReferencePlaceId {
                            source: self.source.clone(),
                            function_span: self.function_span,
                            declaration_span,
                            block_span: declaration_block,
                            binding,
                            projections,
                            place: place.clone(),
                        },
                        creation_span: span,
                        parent: parent_loan.map(Box::new),
                    };
                    let fact = match parent {
                        Some(ReferenceOrigin::BorrowedFrom { parameter, .. }) => {
                            ReferenceOrigin::BorrowedFrom {
                                parameter,
                                loan: loan.clone(),
                            }
                        }
                        Some(ReferenceOrigin::Fresh { allocation, .. }) => ReferenceOrigin::Fresh {
                            allocation,
                            loan: loan.clone(),
                        },
                        None => ReferenceOrigin::Fresh {
                            allocation: ReturnedAllocationId {
                                source: self.source.clone(),
                                function_span: self.function_span,
                                declaration_span,
                                creation_span: span,
                            },
                            loan: loan.clone(),
                        },
                        Some(ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. }) => {
                            unreachable!()
                        }
                    };
                    let id = self.intern(fact);
                    origins.insert(id);
                    self.point(
                        CfgPointKind::Borrow {
                            place: loan.origin_place.clone(),
                            loan,
                        },
                        span,
                        scope,
                        BTreeSet::from([id]),
                    );
                }
                Ok(ReferenceValue::Checked(origins))
            }
            _ => {
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
        }
    }

    fn place_expressions(
        &mut self,
        place: &ScalarPlace,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { name, span } => {
                if self.names.contains_key(name)
                    && self.checked.contains(&self.binding(name, *span)?)
                {
                    let binding = self.binding(name, *span)?;
                    self.point(
                        CfgPointKind::Read { binding },
                        *span,
                        scope,
                        BTreeSet::new(),
                    );
                } else if !self.names.contains_key(name) && !self.module_names.contains(name) {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                }
            }
            ScalarPlace::Dereference { pointer, .. } => {
                self.expression(pointer, scope)?;
                let ScalarPlace::Dereference { span, .. } = place else {
                    unreachable!()
                };
                let point = self.cursor.ok_or_else(|| {
                    self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    )
                })?;
                self.dereference_points.insert(*span, point);
            }
            ScalarPlace::Field { base, .. } => self.place_expressions(base, scope)?,
            ScalarPlace::Index { base, index, .. } => {
                self.place_expressions(base, scope)?;
                self.expression(index, scope)?;
            }
        }
        Ok(())
    }

    fn checked_place_read(
        &mut self,
        place: &ScalarPlace,
        expression: &ScalarExpression,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        if let ScalarPlace::Name { name, .. } = reference_root(place) {
            if self.module_names.contains(name) && !self.names.contains_key(name) {
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                return Ok(ReferenceValue::Other);
            }
        }
        let reads_checked_value = matches!(
            self.indexed_move_type(place)?,
            ScalarType::CheckedReference { .. }
        );
        let reads_checked_aggregate = if let ScalarPlace::Dereference { pointer, .. } =
            reference_root(place)
        {
            matches!(
                pointer.as_ref(),
                ScalarExpression::Name { name, .. }
                    if matches!(self.binding_types.get(&self.binding(name, span)?),
                        Some(ScalarType::CheckedReference { inner, .. })
                            if matches!(inner.as_ref(), ScalarType::Struct(_) | ScalarType::Array { .. }))
            )
        } else {
            false
        };
        if !reads_checked_value
            && !reads_checked_aggregate
            && matches!(reference_root(place), ScalarPlace::Dereference { pointer, .. } if !matches!(pointer.as_ref(), ScalarExpression::Name { .. }))
        {
            self.place_expressions(place, scope)?;
            self.point(
                CfgPointKind::Evaluate(expression.clone()),
                span,
                scope,
                BTreeSet::new(),
            );
            return Ok(ReferenceValue::Other);
        }
        self.place_expressions(place, scope)?;
        let pointer_point = match reference_root(place) {
            ScalarPlace::Dereference { span, .. } => {
                Some(*self.dereference_points.get(span).ok_or_else(|| {
                    self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    )
                })?)
            }
            ScalarPlace::Name { .. } => None,
            ScalarPlace::Field { .. } | ScalarPlace::Index { .. } => unreachable!(),
        };
        let candidates = if pointer_point.is_none() {
            vec![self.assignment_place(place, span)?]
        } else {
            Vec::new()
        };
        let event = CfgPointKind::ReadPlace {
            place: place.clone(),
            pointer_point,
            candidates,
            reads_origin: reads_checked_value,
        };
        if reads_checked_value || reads_checked_aggregate {
            self.uncomputed_point(event, span, scope);
            Ok(ReferenceValue::Uncomputed)
        } else {
            self.point(event, span, scope, BTreeSet::new());
            Ok(ReferenceValue::Other)
        }
    }

    fn place_indices(
        &mut self,
        place: &ScalarPlace,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => {}
            ScalarPlace::Field { base, .. } => self.place_indices(base, scope)?,
            ScalarPlace::Index { base, index, .. } => {
                self.place_indices(base, scope)?;
                self.expression(index, scope)?;
            }
        }
        Ok(())
    }

    fn indexed_move_type(&self, place: &ScalarPlace) -> Result<ScalarType, ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { name, span } => {
                let binding = self.binding(name, *span)?;
                self.binding_types.get(&binding).cloned().ok_or_else(|| {
                    self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    )
                })
            }
            ScalarPlace::Field { base, field, span } => {
                let ScalarType::Struct(id) = self.indexed_move_type(base)? else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                };
                let ScalarFieldReference::Resolved(field) = field else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                };
                self.structs
                    .iter()
                    .find(|structure| structure.id == id)
                    .and_then(|structure| {
                        structure
                            .fields
                            .iter()
                            .find(|candidate| candidate.id == *field)
                    })
                    .map(|field| field.ty.clone())
                    .ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })
            }
            ScalarPlace::Index { base, span, .. } => match self.indexed_move_type(base)? {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    Ok(*element)
                }
                _ => Err(self
                    .missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding)),
            },
            ScalarPlace::Dereference { pointer, span } => {
                let pointer_type = match pointer.as_ref() {
                    ScalarExpression::Name { name, .. } => {
                        self.indexed_move_type(&ScalarPlace::Name {
                            name: name.clone(),
                            span: *span,
                        })?
                    }
                    ScalarExpression::Call {
                        receiver: Some(receiver),
                        name,
                        type_arguments,
                        ..
                    } if receiver == "core"
                        && name == "pointer_cast"
                        && type_arguments.len() == 1 =>
                    {
                        type_arguments[0].ty.clone()
                    }
                    _ => {
                        return Err(self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                        ));
                    }
                };
                match pointer_type {
                    ScalarType::CheckedReference { inner, .. } => Ok(*inner),
                    ScalarType::RawPointer(inner) => Ok(*inner),
                    _ => Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    )),
                }
            }
        }
    }

    fn move_value(
        &mut self,
        value: &ScalarExpression,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        let (name, place, ty, span) = match value {
            ScalarExpression::Name { name, span } => {
                let Some(binding) = self.names.get(name) else {
                    return Ok(());
                };
                (
                    name,
                    ScalarPlace::Name {
                        name: name.clone(),
                        span: *span,
                    },
                    self.binding_types.get(binding).cloned().ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?,
                    *span,
                )
            }
            ScalarExpression::Member {
                receiver,
                name,
                receiver_span,
                span,
                ..
            } => {
                let Some(binding) = self.names.get(receiver) else {
                    return Ok(());
                };
                let Some(receiver_type) = self.binding_types.get(binding) else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                };
                let ScalarType::Struct(id) = receiver_type else {
                    return Ok(());
                };
                let field = self
                    .structs
                    .iter()
                    .find(|item| item.id == *id)
                    .ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?
                    .fields
                    .iter()
                    .find(|field| field.name == *name)
                    .ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                (
                    receiver,
                    ScalarPlace::Field {
                        base: Box::new(ScalarPlace::Name {
                            name: receiver.clone(),
                            span: *receiver_span,
                        }),
                        field: ScalarFieldReference::Resolved(field.id.clone()),
                        span: *span,
                    },
                    field.ty.clone(),
                    *span,
                )
            }
            ScalarExpression::IndexedRead { place, span } => {
                if let ScalarPlace::Name { name, .. } = reference_root(place) {
                    if self.module_names.contains(name) && !self.names.contains_key(name) {
                        return Ok(());
                    }
                }
                let ty = self.indexed_move_type(place)?;
                let policy = copy_policy(&ty, &self.structs, &self.enums, &self.generic_parameters)
                    .map_err(|_| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                match policy {
                    CopyPolicy::Copy => return Ok(()),
                    CopyPolicy::Move => {
                        return Err(self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MoveFromArrayElement,
                        ));
                    }
                    CopyPolicy::Deferred => {
                        self.point(
                            CfgPointKind::DeferredArrayElement {
                                place: place.clone(),
                                ty,
                            },
                            *span,
                            scope,
                            BTreeSet::new(),
                        );
                        return Ok(());
                    }
                }
            }
            ScalarExpression::Dereference { place, span } => {
                let ScalarPlace::Dereference {
                    pointer,
                    span: root_span,
                } = reference_root(place)
                else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                let ty = self.indexed_move_type(place)?;
                let policy = copy_policy(&ty, &self.structs, &self.enums, &self.generic_parameters)
                    .map_err(|_| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                if policy == CopyPolicy::Copy {
                    return Ok(());
                }
                let ScalarExpression::Name { name, .. } = pointer.as_ref() else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                let binding = self.binding(name, *span)?;
                let Some(ScalarType::CheckedReference {
                    mutability: ScalarReferenceMutability::Mutable,
                    ..
                }) = self.binding_types.get(&binding)
                else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                let pointer_point = *self.dereference_points.get(root_span).ok_or_else(|| {
                    self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    )
                })?;
                self.point(
                    if policy == CopyPolicy::Deferred {
                        CfgPointKind::DeferredMoveThrough {
                            place: place.clone(),
                            ty,
                            pointer_point,
                            candidates: Vec::new(),
                        }
                    } else {
                        CfgPointKind::MoveThrough {
                            place: place.clone(),
                            ty,
                            pointer_point,
                            candidates: Vec::new(),
                        }
                    },
                    *span,
                    scope,
                    BTreeSet::new(),
                );
                return Ok(());
            }
            _ => return Ok(()),
        };
        let policy = copy_policy(&ty, &self.structs, &self.enums, &self.generic_parameters)
            .map_err(|_| {
                self.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
            })?;
        if policy == CopyPolicy::Copy {
            return Ok(());
        }
        let binding = self
            .names
            .get(name)
            .expect("validated move binding")
            .clone();
        let mut projections = Vec::new();
        collect_reference_projections(&place, None, &mut projections);
        let place = ReferencePlaceId {
            source: self.source.clone(),
            function_span: self.function_span,
            declaration_span: binding.declaration_span,
            block_span: binding.block_span,
            binding: binding.clone(),
            projections,
            place,
        };
        self.point(
            if policy == CopyPolicy::Deferred {
                CfgPointKind::DeferredMove {
                    place,
                    ty,
                    owner: binding,
                }
            } else {
                CfgPointKind::Move {
                    place,
                    ty,
                    owner: binding,
                }
            },
            span,
            scope,
            BTreeSet::new(),
        );
        Ok(())
    }

    fn additional_receivers(
        &mut self,
        binding: &ScalarBinding,
        block: &ScalarBlock,
        scope: ReferenceScopeId,
        declared: &mut Vec<(String, ReferenceBindingId, Option<ReferenceBindingId>)>,
    ) -> Result<(), ReferenceAnalysisError> {
        for (position, receiver) in binding.receivers.iter().enumerate().skip(1) {
            let value = &binding.output_values[position].value;
            if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions {
                self.expression(value, scope)?;
            } else if matches!(value, ScalarExpression::Call { .. }) {
                let argument_points = self
                    .cfg
                    .points
                    .iter()
                    .rev()
                    .find_map(|point| match &point.kind {
                        CfgPointKind::Call {
                            expression,
                            output: 0,
                            argument_points,
                            ..
                        } if expression == value => Some(argument_points.clone()),
                        _ => None,
                    })
                    .expect("evaluated multi-output call");
                self.uncomputed_point(
                    CfgPointKind::Call {
                        expression: value.clone(),
                        output: position,
                        target: self.call_target(value)?,
                        argument_points,
                    },
                    expression_span(value),
                    scope,
                );
            }
            let id = ReferenceBindingId {
                source: self.source.clone(),
                function_span: self.function_span,
                declaration_span: receiver.name_span,
                block_span: block.span,
                kind: ReferenceBindingKind::Declared,
            };
            let previous = self.names.insert(receiver.name.clone(), id.clone());
            self.binding_types.insert(id.clone(), receiver.ty.clone());
            declared.push((receiver.name.clone(), id.clone(), previous));
            if matches!(receiver.ty, ScalarType::CheckedReference { .. }) {
                self.checked.insert(id.clone());
                self.uncomputed_point(
                    CfgPointKind::Bind { binding: id },
                    receiver.name_span,
                    scope,
                );
            } else {
                self.point(
                    CfgPointKind::Bind { binding: id },
                    receiver.name_span,
                    scope,
                    BTreeSet::new(),
                );
            }
        }
        Ok(())
    }

    fn block(
        &mut self,
        block: &ScalarBlock,
    ) -> Result<Option<ReferenceValue>, ReferenceAnalysisError> {
        let scope = ReferenceScopeId(self.next_scope);
        self.next_scope += 1;
        self.point(CfgPointKind::ScopeEnter, block.span, scope, BTreeSet::new());
        let mut declared = Vec::new();
        let mut result = None;
        let mut return_values = Vec::new();
        for (index, item) in block.items.iter().enumerate() {
            match item {
                ScalarBlockItem::LocalBinding(binding) => {
                    let value = if matches!(binding.declared_type, ScalarType::RawPointer(_))
                        && matches!(&binding.value, ScalarExpression::Name { name, .. } if name == "null")
                    {
                        self.point(
                            CfgPointKind::Evaluate(binding.value.clone()),
                            expression_span(&binding.value),
                            scope,
                            BTreeSet::new(),
                        );
                        ReferenceValue::Other
                    } else {
                        self.expression(&binding.value, scope)?
                    };
                    self.move_value(&binding.value, scope)?;
                    let id = ReferenceBindingId {
                        source: self.source.clone(),
                        function_span: self.function_span,
                        declaration_span: binding.name_span,
                        block_span: block.span,
                        kind: ReferenceBindingKind::Declared,
                    };
                    self.binding_types
                        .insert(id.clone(), binding.declared_type.clone());
                    if binding.is_allocation {
                        self.cfg.allocations.insert(id.clone());
                    }
                    let origins = match value {
                        ReferenceValue::Checked(origins) => Some(origins),
                        ReferenceValue::Other
                            if matches!(
                                binding.declared_type,
                                ScalarType::CheckedReference { .. }
                            ) =>
                        {
                            return Err(self.missing_origin(
                                binding.name_span,
                                id,
                                ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            ));
                        }
                        ReferenceValue::Other => None,
                        ReferenceValue::Uncomputed
                            if matches!(
                                binding.declared_type,
                                ScalarType::CheckedReference { .. }
                            ) =>
                        {
                            self.checked.insert(id.clone());
                            self.origins.remove(&id);
                            self.uncomputed_point(
                                CfgPointKind::Bind {
                                    binding: id.clone(),
                                },
                                binding.name_span,
                                scope,
                            );
                            let old = self.names.insert(binding.name.clone(), id.clone());
                            declared.push((binding.name.clone(), id, old));
                            self.additional_receivers(binding, block, scope, &mut declared)?;
                            continue;
                        }
                        ReferenceValue::Uncomputed => None,
                    };
                    let old = self.names.insert(binding.name.clone(), id.clone());
                    if let Some(origins) = origins {
                        self.checked.insert(id.clone());
                        self.origins.insert(id.clone(), origins.clone());
                        self.point(
                            CfgPointKind::Bind {
                                binding: id.clone(),
                            },
                            binding.name_span,
                            scope,
                            origins,
                        );
                    } else {
                        self.point(
                            CfgPointKind::Bind {
                                binding: id.clone(),
                            },
                            binding.name_span,
                            scope,
                            BTreeSet::new(),
                        );
                    }
                    declared.push((binding.name.clone(), id, old));
                    self.additional_receivers(binding, block, scope, &mut declared)?;
                }
                ScalarBlockItem::Expression(expression) => {
                    let value = self.expression(expression, scope)?;
                    let value_point = self.cursor.expect("evaluated output expression");
                    self.move_value(expression, scope)?;
                    if index >= block.items.len() - block.final_output_values.len() {
                        return_values.push(value_point);
                        if matches!(value, ReferenceValue::Checked(_)) {
                            result = Some(value);
                        } else if matches!(value, ReferenceValue::Uncomputed)
                            && matches!(
                                block.final_output_values
                                    [index - (block.items.len() - block.final_output_values.len())]
                                .ty,
                                ScalarType::CheckedReference { .. }
                            )
                        {
                            result = Some(value);
                        }
                    }
                }
                ScalarBlockItem::Assignment(assignment) => {
                    let output_types = self.assignment_output_types(assignment)?;
                    let mut values = Vec::new();
                    for value in &assignment.values {
                        let result = self.expression(value, scope)?;
                        let rhs_point = self.cursor.expect("evaluated assignment output");
                        self.move_value(value, scope)?;
                        values.push((result, rhs_point));
                    }
                    if assignment.values.len() == 1 {
                        if let ScalarExpression::Call { .. } = &assignment.values[0] {
                            let argument_points = self
                                .cfg
                                .points
                                .iter()
                                .rev()
                                .find_map(|point| match &point.kind {
                                    CfgPointKind::Call {
                                        expression,
                                        output: 0,
                                        argument_points,
                                        ..
                                    } if expression == &assignment.values[0] => {
                                        Some(argument_points.clone())
                                    }
                                    _ => None,
                                })
                                .expect("evaluated assignment call");
                            for position in 1..assignment.targets.len() {
                                let rhs_point = self.point_with_origins(
                                    CfgPointKind::Call {
                                        expression: assignment.values[0].clone(),
                                        output: position,
                                        target: self.call_target(&assignment.values[0])?,
                                        argument_points: argument_points.clone(),
                                    },
                                    expression_span(&assignment.values[0]),
                                    scope,
                                    None,
                                );
                                values.push((ReferenceValue::Uncomputed, rhs_point));
                            }
                        }
                    }
                    for (position, (target, (value, rhs_point))) in
                        assignment.targets.iter().zip(values).enumerate()
                    {
                        if let ScalarPlace::Name { name, span } = &target.place {
                            let id = if let Some(id) = self.names.get(name) {
                                id.clone()
                            } else if self.module_names.contains(name) {
                                if matches!(value, ReferenceValue::Checked(_)) {
                                    return Err(self.missing_context(
                                        *span,
                                        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                    ));
                                }
                                self.point(
                                    CfgPointKind::Evaluate(ScalarExpression::Name {
                                        name: name.clone(),
                                        span: *span,
                                    }),
                                    *span,
                                    scope,
                                    BTreeSet::new(),
                                );
                                continue;
                            } else {
                                let id = ReferenceBindingId {
                                    source: self.source.clone(),
                                    function_span: self.function_span,
                                    declaration_span: target.target_span,
                                    block_span: block.span,
                                    kind: ReferenceBindingKind::Declared,
                                };
                                self.names.insert(name.clone(), id.clone());
                                let ty = output_types.get(position).ok_or_else(|| {
                                    self.missing_context(
                                        target.target_span,
                                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                    )
                                })?;
                                self.binding_types.insert(id.clone(), ty.clone());
                                declared.push((name.clone(), id.clone(), None));
                                id
                            };
                            let target_place =
                                self.assignment_place(&target.place, target.target_span)?;
                            let previous_origins =
                                self.origins.get(&id).cloned().unwrap_or_default();
                            let previous_loans = previous_origins
                                .iter()
                                .filter_map(|origin| match &self.cfg.facts[origin.0] {
                                    ReferenceOrigin::Fresh { loan, .. }
                                    | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                                        Some(loan.clone())
                                    }
                                    ReferenceOrigin::Null { .. }
                                    | ReferenceOrigin::Invalid { .. } => None,
                                })
                                .collect();
                            let assign = CfgPointKind::Assign {
                                target: target_place,
                                rhs_point,
                                previous_origins,
                                previous_loans,
                            };
                            if let ReferenceValue::Checked(origins) = value {
                                self.checked.insert(id.clone());
                                self.origins.insert(id.clone(), origins.clone());
                                self.point(assign, *span, scope, origins);
                            } else if matches!(value, ReferenceValue::Uncomputed)
                                && self.checked.contains(&id)
                            {
                                self.checked.insert(id.clone());
                                self.origins.remove(&id);
                                self.uncomputed_point(assign, *span, scope);
                            } else {
                                if self.checked.contains(&id) {
                                    return Err(self.missing_origin(
                                        *span,
                                        id,
                                        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                    ));
                                }
                                self.origins.remove(&id);
                                self.point(assign, *span, scope, BTreeSet::new());
                            }
                        } else {
                            if let ScalarPlace::Name { name, span } = reference_root(&target.place)
                            {
                                if self.module_names.contains(name)
                                    && !self.names.contains_key(name)
                                {
                                    self.point(
                                        CfgPointKind::Evaluate(ScalarExpression::Name {
                                            name: name.clone(),
                                            span: *span,
                                        }),
                                        target.target_span,
                                        scope,
                                        BTreeSet::new(),
                                    );
                                    continue;
                                }
                            }
                            self.place_expressions(&target.place, scope)?;
                            if let ScalarPlace::Dereference { span, .. } =
                                reference_root(&target.place)
                            {
                                let pointer_point = *self.dereference_points.get(span).ok_or_else(|| {
                                    self.missing_context(
                                        target.target_span,
                                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                    )
                                })?;
                                let assign = CfgPointKind::AssignThrough {
                                    place: target.place.clone(),
                                    pointer_point,
                                    rhs_point,
                                    candidates: Vec::new(),
                                    previous_origins: BTreeSet::new(),
                                    previous_loans: Vec::new(),
                                };
                                match value {
                                    ReferenceValue::Checked(origins) => {
                                        self.point(assign, target.target_span, scope, origins);
                                    }
                                    ReferenceValue::Uncomputed
                                        if matches!(
                                            output_types[position],
                                            ScalarType::CheckedReference { .. }
                                        ) =>
                                    {
                                        self.uncomputed_point(assign, target.target_span, scope);
                                    }
                                    _ => {
                                        self.point(
                                            assign,
                                            target.target_span,
                                            scope,
                                            BTreeSet::new(),
                                        );
                                    }
                                }
                            } else {
                                let target_place =
                                    self.assignment_place(&target.place, target.target_span)?;
                                let assign = CfgPointKind::Assign {
                                    target: target_place,
                                    rhs_point,
                                    previous_origins: BTreeSet::new(),
                                    previous_loans: Vec::new(),
                                };
                                match value {
                                    ReferenceValue::Checked(origins) => {
                                        self.point(assign, target.target_span, scope, origins);
                                    }
                                    ReferenceValue::Uncomputed
                                        if matches!(
                                            output_types[position],
                                            ScalarType::CheckedReference { .. }
                                        ) =>
                                    {
                                        self.uncomputed_point(assign, target.target_span, scope);
                                    }
                                    _ => {
                                        self.point(
                                            assign,
                                            target.target_span,
                                            scope,
                                            BTreeSet::new(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                ScalarBlockItem::While(loop_) => {
                    let loop_head = self.point(
                        CfgPointKind::LoopTest {
                            condition: loop_.condition.clone(),
                        },
                        expression_span(&loop_.condition),
                        scope,
                        BTreeSet::new(),
                    );
                    self.expression(&loop_.condition, scope)?;
                    let branch = self.point(
                        CfgPointKind::Branch {
                            condition: loop_.condition.clone(),
                        },
                        expression_span(&loop_.condition),
                        scope,
                        BTreeSet::new(),
                    );
                    let names = self.names.clone();
                    let origins = self.origins.clone();
                    let checked = self.checked.clone();
                    self.block(&loop_.body)?;
                    let back = self.cursor.expect("loop body terminal");
                    self.cfg.points[back.0].successors.push(loop_head);
                    self.names = names;
                    self.origins = origins;
                    self.checked = checked;
                    self.cursor = Some(branch);
                    self.point(CfgPointKind::Join, loop_.span, scope, BTreeSet::new());
                }
            }
        }
        if scope == ReferenceScopeId(0) {
            for (output, value_point) in return_values.into_iter().enumerate() {
                self.uncomputed_point(
                    CfgPointKind::ReturnOutput {
                        output,
                        value_point,
                        checked: matches!(
                            block.final_output_values[output].ty,
                            ScalarType::CheckedReference { .. }
                        ),
                    },
                    block.final_output_values[output].span,
                    scope,
                );
            }
        }
        self.point(CfgPointKind::ScopeExit, block.span, scope, BTreeSet::new());
        if scope == ReferenceScopeId(0) && !block.final_output_values.is_empty() {
            let return_span = ByteSpan::new(
                block
                    .final_output_values
                    .first()
                    .expect("output")
                    .span
                    .start,
                block.final_output_values.last().expect("output").span.end,
            );
            if let Some(value) = &result {
                match value {
                    ReferenceValue::Checked(origins) => {
                        self.point(CfgPointKind::Return, return_span, scope, origins.clone());
                    }
                    ReferenceValue::Uncomputed => {
                        self.uncomputed_point(CfgPointKind::Return, return_span, scope);
                    }
                    ReferenceValue::Other => {
                        self.point(CfgPointKind::Return, return_span, scope, BTreeSet::new());
                    }
                }
            } else {
                self.point(CfgPointKind::Return, return_span, scope, BTreeSet::new());
            }
        }
        for (name, id, previous) in declared.into_iter().rev() {
            self.origins.remove(&id);
            self.checked.remove(&id);
            self.binding_types.remove(&id);
            if let Some(previous) = previous {
                self.names.insert(name, previous);
            } else {
                self.names.remove(&name);
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Default, PartialEq)]
struct ReferenceEdgeState {
    bindings: BTreeMap<ReferenceBindingId, BTreeSet<ReferenceOriginId>>,
    projected: Vec<ReferenceProjectedState>,
    uninitialized: HashSet<ReferenceBindingId>,
    value: Option<BTreeSet<ReferenceOriginId>>,
    released: BTreeSet<ReferenceBindingId>,
    invalidated: BTreeSet<ReferenceBindingId>,
    point_values: Vec<Option<BTreeSet<ReferenceOriginId>>>,
    return_outputs: BTreeMap<usize, BTreeSet<ReferenceOriginId>>,
    moved: Vec<(ReferencePlaceId, SourceSpan)>,
}

#[derive(Clone, PartialEq)]
struct ReferenceProjectedState {
    place: ReferencePlaceId,
    origins: BTreeSet<ReferenceOriginId>,
    uninitialized: bool,
}

fn reference_places_same(left: &ReferencePlaceId, right: &ReferencePlaceId) -> bool {
    left.binding == right.binding
        && left.projections.len() == right.projections.len()
        && left
            .projections
            .iter()
            .zip(&right.projections)
            .all(|(left, right)| match (left, right) {
                (
                    ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(left)),
                    ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(right)),
                ) => left == right,
                (
                    ReferencePlaceProjection::Index(ScalarExpression::Integer {
                        value: left, ..
                    }),
                    ReferencePlaceProjection::Index(ScalarExpression::Integer {
                        value: right, ..
                    }),
                ) => left == right,
                (
                    ReferencePlaceProjection::Dereference(left),
                    ReferencePlaceProjection::Dereference(right),
                ) => left == right,
                _ => false,
            })
}

fn available_place(place: &ReferencePlaceId) -> ReferencePlaceId {
    let mut resolved = place.clone();
    let mut projections = Vec::new();
    for projection in &place.projections {
        match projection {
            ReferencePlaceProjection::Dereference(loan) => {
                let owner = available_place(&loan.origin_place);
                resolved.binding = owner.binding;
                projections = owner.projections;
            }
            projection => projections.push(projection.clone()),
        }
    }
    resolved.projections = projections;
    resolved
}

fn unavailable_place(state: &ReferenceEdgeState, place: &ReferencePlaceId) -> bool {
    let place = available_place(place);
    state
        .moved
        .iter()
        .any(|(moved, _)| reference_places_overlap(moved, &place))
}

fn unavailable_origins(state: &ReferenceEdgeState, place: &ReferencePlaceId) -> Vec<SourceSpan> {
    let place = available_place(place);
    state
        .moved
        .iter()
        .filter(|(moved, _)| reference_places_overlap(moved, &place))
        .map(|(_, origin)| origin.clone())
        .collect()
}

fn reference_read_reachable(
    points: &[ReferenceFlowPoint],
    from: CfgPointId,
    binding: &ReferenceBindingId,
) -> bool {
    let mut pending = points[from.0].successors.clone();
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id.0) {
            continue;
        }
        if matches!(&points[id.0].kind, CfgPointKind::Read { binding: read } if read == binding) {
            return true;
        }
        if matches!(&points[id.0].kind, CfgPointKind::Assign { target, .. } if target.binding == *binding && target.projections.is_empty())
            || matches!(&points[id.0].kind, CfgPointKind::Bind { binding: bound } if bound == binding)
            || matches!(&points[id.0].kind, CfgPointKind::ScopeExit if points[id.0].source_span.range == binding.block_span)
        {
            continue;
        }
        pending.extend(&points[id.0].successors);
    }
    false
}

fn loan_borrows_place(loan: &ReferenceLoanId, place: &ReferencePlaceId) -> bool {
    reference_places_overlap(&available_place(&loan.origin_place), place)
        || loan
            .parent
            .as_ref()
            .is_some_and(|parent| loan_borrows_place(parent, place))
}

fn conflicting_live_loan(
    points: &[ReferenceFlowPoint],
    facts: &[ReferenceOrigin],
    state: &ReferenceEdgeState,
    point: CfgPointId,
    place: &ReferencePlaceId,
    consuming_origin: Option<ReferenceOriginId>,
) -> Vec<SourceSpan> {
    let place = available_place(place);
    state
        .bindings
        .iter()
        .filter(|(binding, _)| reference_read_reachable(points, point, binding))
        .flat_map(|(_, origins)| origins.iter())
        .filter_map(|origin| {
            if Some(*origin) == consuming_origin {
                return None;
            }
            match &facts[origin.0] {
                ReferenceOrigin::Fresh { loan, .. }
                | ReferenceOrigin::BorrowedFrom { loan, .. }
                    if loan_borrows_place(loan, &place) =>
                {
                    Some(SourceSpan::new(
                        loan.origin_place.source.clone(),
                        loan.creation_span,
                    ))
                }
                _ => None,
            }
        })
        .collect()
}

fn write_projected_reference(
    state: &mut ReferenceEdgeState,
    place: &ReferencePlaceId,
    origins: &BTreeSet<ReferenceOriginId>,
) {
    state
        .projected
        .retain(|known| !reference_places_overlap(&known.place, place));
    state.projected.push(ReferenceProjectedState {
        place: place.clone(),
        origins: origins.clone(),
        uninitialized: false,
    });
}

fn transfer_reference_facts(
    cfg: &mut ScalarReferenceCfg,
    deferred_policies: &BTreeMap<usize, CopyPolicy>,
    contracts: &[ConcreteReferenceOutputContract],
) -> Vec<ReferenceAnalysisError> {
    let original_points = cfg.points.clone();
    let mut last_use = (0..cfg.points.len()).collect::<Vec<_>>();
    for point in &original_points {
        let dependencies = match &point.kind {
            CfgPointKind::Invalidate { address_point, .. }
            | CfgPointKind::Release {
                target: ReleaseTarget::Checked { address_point, .. },
            } => vec![*address_point],
            CfgPointKind::MoveThrough { pointer_point, .. }
            | CfgPointKind::DeferredMoveThrough { pointer_point, .. } => vec![*pointer_point],
            CfgPointKind::Assign { rhs_point, .. } => vec![*rhs_point],
            CfgPointKind::ReturnOutput { value_point, .. } => vec![*value_point],
            CfgPointKind::Call {
                argument_points, ..
            } => argument_points.clone(),
            CfgPointKind::AssignThrough {
                rhs_point,
                pointer_point,
                ..
            } => {
                vec![*rhs_point, *pointer_point]
            }
            CfgPointKind::ReadPlace { pointer_point, .. } => {
                pointer_point.iter().copied().collect()
            }
            _ => Vec::new(),
        };
        for dependency in dependencies {
            last_use[dependency.0] = point.id.0;
        }
    }
    let mut observed = vec![None::<ReferenceFlowPoint>; cfg.points.len()];
    let mut incoming = vec![Vec::<ReferenceEdgeState>::new(); cfg.points.len()];
    incoming[0].push(ReferenceEdgeState {
        point_values: vec![None; cfg.points.len()],
        ..ReferenceEdgeState::default()
    });
    let mut visited = vec![Vec::<ReferenceEdgeState>::new(); cfg.points.len()];
    let mut queued = vec![false; cfg.points.len()];
    queued[0] = true;
    let mut ready = std::collections::VecDeque::from([CfgPointId(0)]);
    let mut errors = Vec::new();
    while let Some(id) = ready.pop_front() {
        queued[id.0] = false;
        let multiple_paths = incoming[id.0].len() > 1;
        for mut state in std::mem::take(&mut incoming[id.0]) {
            if visited[id.0].contains(&state) {
                continue;
            }
            visited[id.0].push(state.clone());
            let mut move_alternatives = Vec::new();
            cfg.points[id.0] = original_points[id.0].clone();
            let invalidation_address = match &cfg.points[id.0].kind {
                CfgPointKind::Invalidate {
                    address_point,
                    address_span,
                    ..
                } => Some((
                    address_span.clone(),
                    state.point_values[address_point.0].clone(),
                )),
                CfgPointKind::Release {
                    target:
                        ReleaseTarget::Checked {
                            address_point,
                            address_span,
                            ..
                        },
                } => Some((
                    address_span.clone(),
                    state.point_values[address_point.0].clone(),
                )),
                _ => None,
            };
            let move_pointer = match &cfg.points[id.0].kind {
                CfgPointKind::MoveThrough { pointer_point, .. }
                | CfgPointKind::DeferredMoveThrough { pointer_point, .. } => {
                    Some(state.point_values[pointer_point.0].clone())
                }
                _ => None,
            };
            let assignment_rhs = match &cfg.points[id.0].kind {
                CfgPointKind::Assign { rhs_point, .. }
                | CfgPointKind::AssignThrough { rhs_point, .. } => {
                    Some(state.point_values[rhs_point.0].clone())
                }
                _ => None,
            };
            let assignment_pointer = match &cfg.points[id.0].kind {
                CfgPointKind::AssignThrough { pointer_point, .. } => {
                    Some(state.point_values[pointer_point.0].clone())
                }
                _ => None,
            };
            let place_read_pointer = match &cfg.points[id.0].kind {
                CfgPointKind::ReadPlace { pointer_point, .. } => {
                    pointer_point.map(|pointer_point| state.point_values[pointer_point.0].clone())
                }
                _ => None,
            };
            let call_origins = if let CfgPointKind::Call {
                expression: ScalarExpression::Call { type_arguments, .. },
                output,
                target:
                    ResolvedCallTarget::ModuleCallable {
                        source,
                        declaration_span,
                        concrete,
                    },
                argument_points,
                ..
            } = &cfg.points[id.0].kind
            {
                let contract = contracts.iter().find(|contract| {
                    contract.source == *source
                        && contract.declaration_span == *declaration_span
                        && contract.selection == *concrete
                        && contract.type_arguments
                            == type_arguments
                                .iter()
                                .map(|argument| argument.ty.clone())
                                .collect::<Vec<_>>()
                        && contract.output == *output
                });
                contract.map(|contract| {
                    let mut results = BTreeSet::new();
                    for origin in &contract.origins {
                        let actual = match origin {
                            ReferenceOrigin::BorrowedFrom { parameter, .. } => {
                                if let Some(argument) = state.point_values[argument_points[*parameter].0].as_ref() {
                                    results.extend(argument);
                                } else {
                                    errors.push(ReferenceAnalysisError::MissingContext {
                                        source_span: cfg.points[id.0].source_span.clone(),
                                        reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                    });
                                }
                                continue;
                            }
                            ReferenceOrigin::Fresh { allocation, loan } => {
                                let mut loan = loan.clone();
                                let binding = ReferenceBindingId {
                                    source: cfg.points[id.0].source_span.source.clone(),
                                    function_span: cfg.points[0].source_span.range,
                                    declaration_span: cfg.points[id.0].source_span.range,
                                    block_span: cfg.points[0].source_span.range,
                                    kind: ReferenceBindingKind::CallOutput {
                                        point: id,
                                        output: *output,
                                    },
                                };
                                loan.origin_place.source = binding.source.clone();
                                loan.origin_place.function_span = binding.function_span;
                                loan.origin_place.declaration_span = binding.declaration_span;
                                loan.origin_place.block_span = binding.block_span;
                                loan.origin_place.binding = binding;
                                loan.creation_span = cfg.points[id.0].source_span.range;
                                ReferenceOrigin::Fresh {
                                    allocation: allocation.clone(),
                                    loan,
                                }
                            }
                            ReferenceOrigin::Null { .. } => ReferenceOrigin::Null {
                                span: cfg.points[id.0].source_span.range,
                            },
                            ReferenceOrigin::Invalid {
                                origin_span,
                                conflict_span,
                            } => ReferenceOrigin::Invalid {
                                origin_span: *origin_span,
                                conflict_span: *conflict_span,
                            },
                        };
                        let origin_id = if let Some(index) = cfg.facts.iter().position(|fact| fact == &actual) {
                            ReferenceOriginId(index)
                        } else {
                            let index = cfg.facts.len();
                            cfg.facts.push(actual);
                            ReferenceOriginId(index)
                        };
                        results.insert(origin_id);
                    }
                    results
                })
            } else {
                None
            };
            let point = &mut cfg.points[id.0];
            if let Some(origins) = call_origins {
                point.possible_origins = Some(origins);
            }
            if let CfgPointKind::ReadPlace {
                place,
                candidates,
                reads_origin,
                ..
            } = &mut point.kind
            {
                if let Some(origins) = place_read_pointer {
                    if let Some(origins) = origins.filter(|origins| !origins.is_empty()) {
                        if !*reads_origin {
                            point.possible_origins = Some(origins.clone());
                        }
                        for origin in origins {
                            match &cfg.facts[origin.0] {
                                ReferenceOrigin::Fresh { loan, .. }
                                | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                                    let mut resolved = loan.origin_place.clone();
                                    resolved.place = place.clone();
                                    collect_reference_projections(
                                        place,
                                        Some(loan),
                                        &mut resolved.projections,
                                    );
                                    candidates.push(resolved);
                                }
                                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                    if *reads_origin {
                                        errors.push(ReferenceAnalysisError::MissingContext {
                                            source_span: point.source_span.clone(),
                                            reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                        });
                                    }
                                }
                            }
                        }
                    } else {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: point.source_span.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                        });
                    }
                }
            }
            if let Some(origins) = assignment_pointer {
                if let Some(origins) = origins.filter(|origins| !origins.is_empty()) {
                    let CfgPointKind::AssignThrough {
                        place, candidates, ..
                    } = &mut point.kind
                    else {
                        unreachable!()
                    };
                    for origin in &origins {
                        let loan = match &cfg.facts[origin.0] {
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason:
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            });
                                continue;
                            }
                        };
                        let mut resolved_place = loan.origin_place.clone();
                        resolved_place.place = place.clone();
                        collect_reference_projections(place, None, &mut resolved_place.projections);
                        candidates.push(InvalidationCandidate {
                            origin: *origin,
                            place: resolved_place,
                            loan: loan.clone(),
                        });
                    }
                } else {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    });
                }
            }
            if let Some(origins) = move_pointer {
                match origins.filter(|origins| !origins.is_empty()) {
                    Some(origins) => {
                        let (place, ty) = match &point.kind {
                            CfgPointKind::MoveThrough { place, ty, .. }
                            | CfgPointKind::DeferredMoveThrough { place, ty, .. } => {
                                (place.clone(), ty.clone())
                            }
                            _ => unreachable!(),
                        };
                        let mut candidates = Vec::new();
                        for origin in &origins {
                            let loan = match &cfg.facts[origin.0] {
                                ReferenceOrigin::Fresh { loan, .. }
                                | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                    errors.push(ReferenceAnalysisError::MissingContext {
                                    source_span: point.source_span.clone(),
                                    reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                });
                                    continue;
                                }
                            };
                            let mut resolved_place = loan.origin_place.clone();
                            resolved_place.place = place.clone();
                            collect_reference_projections(
                                &place,
                                Some(loan),
                                &mut resolved_place.projections,
                            );
                            candidates.push(InvalidationCandidate {
                                origin: *origin,
                                place: resolved_place,
                                loan: loan.clone(),
                            });
                        }
                        let all_candidates_resolved = candidates.len() == origins.len();
                        point.possible_origins = Some(origins);
                        if !matches!(place, ScalarPlace::Dereference { .. })
                            && candidates.len() == 1
                            && all_candidates_resolved
                            && !multiple_paths
                            && matches!(point.kind, CfgPointKind::MoveThrough { .. })
                        {
                            let candidate = &candidates[0];
                            point.kind = CfgPointKind::Move {
                                place: candidate.place.clone(),
                                ty,
                                owner: candidate.place.binding.clone(),
                            };
                        } else if let CfgPointKind::MoveThrough {
                            candidates: event_candidates,
                            ..
                        } = &mut point.kind
                        {
                            *event_candidates = candidates;
                        } else if let CfgPointKind::DeferredMoveThrough {
                            candidates: event_candidates,
                            ..
                        } = &mut point.kind
                        {
                            *event_candidates = candidates;
                        }
                    }
                    None => errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    }),
                }
            }
            if let Some((address_span, origins)) = invalidation_address {
                if let Some(origins) = origins.filter(|origins| !origins.is_empty()) {
                    let mut candidates = Vec::new();
                    for origin in &origins {
                        let loan = match &cfg.facts[origin.0] {
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: address_span.clone(),
                                reason:
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            });
                                continue;
                            }
                        };
                        candidates.push(InvalidationCandidate {
                            origin: *origin,
                            place: loan.origin_place.clone(),
                            loan: loan.clone(),
                        });
                    }
                    if let CfgPointKind::Invalidate {
                        candidates: event_candidates,
                        ..
                    } = &mut point.kind
                    {
                        *event_candidates = candidates;
                    } else if let CfgPointKind::Release {
                        target:
                            ReleaseTarget::Checked {
                                candidates: event_candidates,
                                ..
                            },
                    } = &mut point.kind
                    {
                        *event_candidates = candidates;
                    }
                    point.possible_origins = Some(origins);
                } else {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: address_span,
                        reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                    });
                }
            }
            let affected = match &point.kind {
                CfgPointKind::Invalidate { candidates, .. } => Some(candidates.as_slice()),
                CfgPointKind::Release {
                    target: ReleaseTarget::Checked { candidates, .. },
                } => Some(candidates.as_slice()),
                _ => None,
            };
            if let Some(candidates) = affected {
                for candidate in candidates {
                    let origins = conflicting_live_loan(
                        &original_points,
                        &cfg.facts,
                        &state,
                        id,
                        &candidate.place,
                        Some(candidate.origin),
                    );
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins,
                        });
                    }
                }
                if let CfgPointKind::Invalidate { operation, .. } = &point.kind {
                    for binding in candidates.iter().map(|candidate| &candidate.place.binding) {
                        match operation {
                            CoreOperationId::Invalidate => {
                                state.invalidated.insert(binding.clone());
                            }
                            CoreOperationId::Rebind => {
                                state.invalidated.remove(binding);
                            }
                            _ => unreachable!("resolved invalidation operation"),
                        }
                    }
                }
                if let CfgPointKind::Release {
                    target: ReleaseTarget::Checked { address_span, .. },
                } = &point.kind
                {
                    for binding in candidates
                        .iter()
                        .map(|candidate| &candidate.place.binding)
                        .collect::<BTreeSet<_>>()
                    {
                        if !cfg.allocations.contains(binding) {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: address_span.clone(),
                                reason:
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            });
                        } else if !state.released.insert(binding.clone()) {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        }
                    }
                }
                for origins in state.bindings.values_mut() {
                    origins.retain(|origin| {
                        let place = match &cfg.facts[origin.0] {
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. } => &loan.origin_place,
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                return true;
                            }
                        };
                        !candidates.iter().any(|candidate| {
                            place.binding == candidate.place.binding
                                && place.projections.starts_with(&candidate.place.projections)
                        })
                    });
                }
                state.projected.retain(|known| {
                    !candidates
                        .iter()
                        .any(|candidate| reference_places_overlap(&known.place, &candidate.place))
                });
            }
            if let CfgPointKind::Release {
                target: ReleaseTarget::Raw { binding },
            } = &point.kind
            {
                if !state.released.insert(binding.clone()) {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                    });
                }
                for origins in state.bindings.values_mut() {
                    origins.retain(|origin| match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                            &loan.origin_place.binding != binding
                        }
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => true,
                    });
                }
                state
                    .projected
                    .retain(|known| &known.place.binding != binding);
            }
            if let CfgPointKind::Assign {
                target,
                previous_origins,
                previous_loans,
                ..
            } = &mut point.kind
            {
                *previous_origins = if target.projections.is_empty() {
                    state
                        .bindings
                        .get(&target.binding)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    state
                        .bindings
                        .values()
                        .flat_map(|origins| origins.iter().copied())
                        .filter(|origin| match &cfg.facts[origin.0] {
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                                reference_places_overlap(&loan.origin_place, target)
                            }
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
                        })
                        .collect()
                };
                *previous_loans = previous_origins
                    .iter()
                    .filter_map(|origin| match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => Some(loan.clone()),
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => None,
                    })
                    .collect();
            }
            if let CfgPointKind::AssignThrough {
                candidates,
                previous_origins,
                previous_loans,
                ..
            } = &mut point.kind
            {
                *previous_origins = state
                    .bindings
                    .values()
                    .flat_map(|origins| origins.iter().copied())
                    .filter(|origin| match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                            !candidates
                                .iter()
                                .any(|candidate| candidate.origin == *origin)
                                && candidates.iter().any(|candidate| {
                                    reference_places_overlap(&loan.origin_place, &candidate.place)
                                })
                        }
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
                    })
                    .collect();
                *previous_loans = previous_origins
                    .iter()
                    .filter_map(|origin| match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => Some(loan.clone()),
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => None,
                    })
                    .collect();
            }
            let read_places = match &point.kind {
                CfgPointKind::ReadPlace { candidates, .. } => candidates.as_slice(),
                _ => &[],
            };
            for place in read_places {
                if unavailable_place(&state, place)
                    || state.released.contains(&place.binding)
                    || state.invalidated.contains(&place.binding)
                {
                    errors.push(ReferenceAnalysisError::ConflictingPlace {
                        source_span: point.source_span.clone(),
                        origins: unavailable_origins(&state, place),
                    });
                }
            }
            match &point.kind {
                CfgPointKind::Move {
                    place,
                    ty:
                        ScalarType::Struct(_)
                        | ScalarType::Array { .. }
                        | ScalarType::RuntimeArray { .. },
                    ..
                }
                | CfgPointKind::DeferredMove { place, .. }
                    if matches!(point.kind, CfgPointKind::Move { .. })
                        || deferred_policies.get(&id.0) == Some(&CopyPolicy::Move) =>
                {
                    let place = available_place(place);
                    let mut origins = unavailable_origins(&state, &place);
                    origins.extend(conflicting_live_loan(
                        &original_points,
                        &cfg.facts,
                        &state,
                        id,
                        &place,
                        None,
                    ));
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins,
                        });
                    }
                    if !state.moved.iter().any(|(moved, _)| moved == &place) {
                        state.moved.push((place, point.source_span.clone()));
                    }
                }
                CfgPointKind::MoveThrough { candidates, .. }
                | CfgPointKind::DeferredMoveThrough { candidates, .. }
                    if matches!(point.kind, CfgPointKind::MoveThrough { .. })
                        || deferred_policies.get(&id.0) == Some(&CopyPolicy::Move) =>
                {
                    for candidate in candidates {
                        let place = available_place(&candidate.place);
                        let mut origins = unavailable_origins(&state, &place);
                        origins.extend(conflicting_live_loan(
                            &original_points,
                            &cfg.facts,
                            &state,
                            id,
                            &place,
                            Some(candidate.origin),
                        ));
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
                        if !move_alternatives.iter().any(|(moved, _)| moved == &place) {
                            move_alternatives.push((
                                place,
                                SourceSpan::new(
                                    candidate.place.source.clone(),
                                    candidate.loan.creation_span,
                                ),
                            ));
                        }
                    }
                }
                CfgPointKind::Assign { target, .. } => {
                    let target = available_place(target);
                    if !target.projections.is_empty() {
                        let origins = conflicting_live_loan(
                            &original_points,
                            &cfg.facts,
                            &state,
                            id,
                            &target,
                            None,
                        );
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
                    }
                    state.moved.retain(|(moved, _)| {
                        moved.binding != target.binding
                            || !moved.projections.starts_with(&target.projections)
                    });
                }
                CfgPointKind::AssignThrough { candidates, .. } => {
                    for candidate in candidates {
                        let target = available_place(&candidate.place);
                        let origins = conflicting_live_loan(
                            &original_points,
                            &cfg.facts,
                            &state,
                            id,
                            &target,
                            Some(candidate.origin),
                        );
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
                        state.moved.retain(|(moved, _)| {
                            moved.binding != target.binding
                                || !moved.projections.starts_with(&target.projections)
                        });
                    }
                }
                _ => {}
            }
            let checked_value = match &point.kind {
                CfgPointKind::ReturnOutput { value_point, .. } => {
                    state.point_values[value_point.0].clone()
                }
                CfgPointKind::ReadPlace {
                    reads_origin: false,
                    ..
                } => None,
                CfgPointKind::ReadPlace {
                    candidates,
                    reads_origin: true,
                    ..
                } => {
                    let mut read = BTreeSet::new();
                    for candidate in candidates {
                        let matching = state
                            .projected
                            .iter()
                            .filter(|known| {
                                reference_places_same(&known.place, candidate)
                                    && !known.uninitialized
                            })
                            .collect::<Vec<_>>();
                        if matching.is_empty() {
                            errors.push(ReferenceAnalysisError::MissingOrigin {
                                source_span: point.source_span.clone(),
                                binding_id: candidate.binding.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        }
                        for known in matching {
                            read.extend(&known.origins);
                        }
                    }
                    if read.is_empty() {
                        None
                    } else {
                        Some(read)
                    }
                }
                CfgPointKind::Read { binding } => {
                    if state
                        .moved
                        .iter()
                        .any(|(place, _)| &place.binding == binding)
                    {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins: state
                                .moved
                                .iter()
                                .filter(|(place, _)| &place.binding == binding)
                                .map(|(_, origin)| origin.clone())
                                .collect(),
                        });
                    }
                    if state.released.contains(binding) || state.invalidated.contains(binding) {
                        errors.push(ReferenceAnalysisError::MissingOrigin {
                            source_span: point.source_span.clone(),
                            binding_id: binding.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                        });
                    }
                    if point
                        .possible_origins
                        .as_ref()
                        .is_some_and(BTreeSet::is_empty)
                    {
                        None
                    } else {
                        match state.bindings.get(binding).filter(|origins| {
                            !origins.is_empty() && !state.uninitialized.contains(binding)
                        }) {
                            Some(origins) => Some(origins.clone()),
                            None => {
                                errors.push(ReferenceAnalysisError::MissingOrigin {
                                    source_span: point.source_span.clone(),
                                    binding_id: binding.clone(),
                                    reason:
                                        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                });
                                None
                            }
                        }
                    }
                }
                CfgPointKind::Bind { binding }
                    if point.possible_origins.is_none()
                        || point
                            .possible_origins
                            .as_ref()
                            .is_some_and(|set| !set.is_empty()) =>
                {
                    let rhs = if point.possible_origins.is_none() {
                        state.value.as_ref()
                    } else {
                        point.possible_origins.as_ref()
                    };
                    match rhs.filter(|origins| !origins.is_empty()) {
                        Some(origins) => {
                            let origins = origins.clone();
                            state.bindings.insert(binding.clone(), origins.clone());
                            state.uninitialized.remove(binding);
                            Some(origins)
                        }
                        None => {
                            errors.push(ReferenceAnalysisError::MissingOrigin {
                                source_span: point.source_span.clone(),
                                binding_id: binding.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                            None
                        }
                    }
                }
                CfgPointKind::Assign {
                    target,
                    previous_origins,
                    ..
                } => {
                    let binding = &target.binding;
                    if target.projections.is_empty() {
                        state.bindings.remove(binding);
                        state
                            .projected
                            .retain(|known| &known.place.binding != binding);
                        state.uninitialized.remove(binding);
                        state.released.remove(binding);
                        state.invalidated.remove(binding);
                    } else {
                        for origins in state.bindings.values_mut() {
                            origins.retain(|origin| !previous_origins.contains(origin));
                        }
                        state
                            .projected
                            .retain(|known| !reference_places_overlap(&known.place, target));
                    }
                    if point.possible_origins.is_none()
                        || point
                            .possible_origins
                            .as_ref()
                            .is_some_and(|set| !set.is_empty())
                    {
                        match assignment_rhs
                            .flatten()
                            .filter(|origins| !origins.is_empty())
                        {
                            Some(origins) => {
                                if target.projections.is_empty() {
                                    state.bindings.insert(binding.clone(), origins.clone());
                                } else {
                                    write_projected_reference(&mut state, target, &origins);
                                }
                                Some(origins)
                            }
                            None => {
                                errors.push(ReferenceAnalysisError::MissingOrigin {
                                    source_span: point.source_span.clone(),
                                    binding_id: binding.clone(),
                                    reason:
                                        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                });
                                None
                            }
                        }
                    } else {
                        None
                    }
                }
                CfgPointKind::AssignThrough {
                    candidates,
                    previous_origins,
                    ..
                } => {
                    for origins in state.bindings.values_mut() {
                        origins.retain(|origin| !previous_origins.contains(origin));
                    }
                    let rhs = assignment_rhs
                        .flatten()
                        .filter(|origins| !origins.is_empty());
                    for candidate in candidates {
                        state.projected.retain(|known| {
                            !reference_places_overlap(&known.place, &candidate.place)
                        });
                    }
                    if point
                        .possible_origins
                        .as_ref()
                        .is_none_or(|origins| !origins.is_empty())
                    {
                        if let Some(origins) = &rhs {
                            for candidate in candidates {
                                write_projected_reference(&mut state, &candidate.place, origins);
                            }
                        } else {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        }
                    }
                    rhs
                }
                CfgPointKind::Evaluate(_)
                | CfgPointKind::Borrow { .. }
                | CfgPointKind::Call { .. } => {
                    if let CfgPointKind::Borrow { place, .. } = &point.kind {
                        if state.released.contains(&place.binding) {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        }
                    }
                    if point.possible_origins.is_none() {
                        state.value.clone()
                    } else {
                        point.possible_origins.clone().filter(|set| !set.is_empty())
                    }
                }
                CfgPointKind::Join if point.possible_origins.is_none() => state.value.clone(),
                CfgPointKind::Return => Some(
                    state
                        .return_outputs
                        .values()
                        .flat_map(|origins| origins.iter().copied())
                        .collect(),
                ),
                _ => None,
            };
            if matches!(point.kind, CfgPointKind::ReturnOutput { checked: true, .. })
                && checked_value.as_ref().is_none_or(BTreeSet::is_empty)
            {
                errors.push(ReferenceAnalysisError::MissingContext {
                    source_span: point.source_span.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                });
            }
            if let (CfgPointKind::ReturnOutput { checked: true, .. }, Some(origins)) =
                (&point.kind, &checked_value)
            {
                for origin in origins {
                    let loan = match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => continue,
                    };
                    if unavailable_place(&state, &loan.origin_place)
                        || state.released.contains(&loan.origin_place.binding)
                        || state.invalidated.contains(&loan.origin_place.binding)
                    {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins: vec![SourceSpan::new(
                                loan.origin_place.source.clone(),
                                loan.creation_span,
                            )],
                        });
                    }
                }
            }
            if let (CfgPointKind::ReturnOutput { output, .. }, Some(origins)) =
                (&point.kind, &checked_value)
            {
                if !origins.is_empty() {
                    state.return_outputs.insert(*output, origins.clone());
                }
            }
            if matches!(
                point.kind,
                CfgPointKind::Read { .. }
                    | CfgPointKind::ReadPlace {
                        reads_origin: true,
                        ..
                    }
                    | CfgPointKind::Bind { .. }
                    | CfgPointKind::Assign { .. }
                    | CfgPointKind::AssignThrough { .. }
                    | CfgPointKind::ReturnOutput { .. }
                    | CfgPointKind::Return
            ) || point.possible_origins.is_none()
            {
                point.possible_origins = checked_value.clone();
            }
            match &point.kind {
                CfgPointKind::ScopeExit => {
                    state
                        .bindings
                        .retain(|binding, _| binding.block_span != point.source_span.range);
                    state
                        .uninitialized
                        .retain(|binding| binding.block_span != point.source_span.range);
                    state
                        .moved
                        .retain(|(place, _)| place.binding.block_span != point.source_span.range);
                    state
                        .released
                        .retain(|binding| binding.block_span != point.source_span.range);
                    state
                        .invalidated
                        .retain(|binding| binding.block_span != point.source_span.range);
                    state
                        .projected
                        .retain(|known| known.place.binding.block_span != point.source_span.range);
                }
                CfgPointKind::Read { .. }
                | CfgPointKind::ReadPlace { .. }
                | CfgPointKind::Join
                | CfgPointKind::Evaluate(_)
                | CfgPointKind::Call { .. }
                | CfgPointKind::Borrow { .. }
                | CfgPointKind::Bind { .. }
                | CfgPointKind::Assign { .. } => state.value = checked_value,
                CfgPointKind::AssignThrough { .. } => state.value = checked_value,
                _ => {}
            }
            state.point_values[id.0] = point.possible_origins.clone();
            for (index, value) in state.point_values.iter_mut().enumerate().take(id.0 + 1) {
                if last_use[index] <= id.0 {
                    *value = None;
                }
            }
            if let Some(previous) = &mut observed[id.0] {
                if let Some(origins) = &point.possible_origins {
                    previous
                        .possible_origins
                        .get_or_insert_with(BTreeSet::new)
                        .extend(origins);
                }
                match (&mut previous.kind, &point.kind) {
                    (
                        CfgPointKind::Release {
                            target:
                                ReleaseTarget::Checked {
                                    candidates: known, ..
                                },
                        },
                        CfgPointKind::Release {
                            target: ReleaseTarget::Checked { candidates, .. },
                        },
                    )
                    | (
                        CfgPointKind::Invalidate {
                            candidates: known, ..
                        },
                        CfgPointKind::Invalidate { candidates, .. },
                    )
                    | (
                        CfgPointKind::AssignThrough {
                            candidates: known, ..
                        },
                        CfgPointKind::AssignThrough { candidates, .. },
                    )
                    | (
                        CfgPointKind::MoveThrough {
                            candidates: known, ..
                        },
                        CfgPointKind::MoveThrough { candidates, .. },
                    )
                    | (
                        CfgPointKind::DeferredMoveThrough {
                            candidates: known, ..
                        },
                        CfgPointKind::DeferredMoveThrough { candidates, .. },
                    ) => {
                        for candidate in candidates {
                            if !known.contains(candidate) {
                                known.push(candidate.clone());
                            }
                        }
                    }
                    (
                        CfgPointKind::ReadPlace {
                            candidates: known, ..
                        },
                        CfgPointKind::ReadPlace { candidates, .. },
                    ) => {
                        for candidate in candidates {
                            if !known.contains(candidate) {
                                known.push(candidate.clone());
                            }
                        }
                    }
                    (
                        CfgPointKind::Assign {
                            previous_origins: known,
                            previous_loans: loans,
                            ..
                        },
                        CfgPointKind::Assign {
                            previous_origins,
                            previous_loans,
                            ..
                        },
                    ) => {
                        known.extend(previous_origins);
                        for loan in previous_loans {
                            if !loans.contains(loan) {
                                loans.push(loan.clone());
                            }
                        }
                    }
                    _ => {}
                }
                if let (
                    CfgPointKind::AssignThrough {
                        previous_origins: known,
                        previous_loans: loans,
                        ..
                    },
                    CfgPointKind::AssignThrough {
                        previous_origins,
                        previous_loans,
                        ..
                    },
                ) = (&mut previous.kind, &point.kind)
                {
                    known.extend(previous_origins);
                    for loan in previous_loans {
                        if !loans.contains(loan) {
                            loans.push(loan.clone());
                        }
                    }
                }
            } else {
                observed[id.0] = Some(point.clone());
            }
            for successor in &point.successors {
                let slot = &mut incoming[successor.0];
                if move_alternatives.is_empty() {
                    let mut next = state.clone();
                    if successor.0 <= id.0 {
                        next.point_values[successor.0..].fill(None);
                    }
                    if !visited[successor.0].contains(&next) && !slot.contains(&next) {
                        slot.push(next);
                    }
                } else {
                    for (place, origin) in &move_alternatives {
                        let mut next = state.clone();
                        if !next.moved.iter().any(|(moved, _)| moved == place) {
                            next.moved.push((place.clone(), origin.clone()));
                        }
                        if successor.0 <= id.0 {
                            next.point_values[successor.0..].fill(None);
                        }
                        if !visited[successor.0].contains(&next) && !slot.contains(&next) {
                            slot.push(next);
                        }
                    }
                }
                if !slot.is_empty() && !queued[successor.0] {
                    queued[successor.0] = true;
                    ready.push_back(*successor);
                }
            }
        }
    }
    for (point, observed) in cfg.points.iter_mut().zip(observed) {
        if let Some(observed) = observed {
            *point = observed;
        }
    }
    let mut unique = Vec::new();
    for error in errors {
        if !unique.contains(&error) {
            unique.push(error);
        }
    }
    unique
}

fn reference_root(place: &ScalarPlace) -> &ScalarPlace {
    match place {
        ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => place,
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => reference_root(base),
    }
}

fn collect_reference_projections(
    place: &ScalarPlace,
    parent: Option<&ReferenceLoanId>,
    projections: &mut Vec<ReferencePlaceProjection>,
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { .. } => {
            if let Some(parent) = parent {
                projections.push(ReferencePlaceProjection::Dereference(Box::new(
                    parent.clone(),
                )));
            }
        }
        ScalarPlace::Field { base, field, .. } => {
            collect_reference_projections(base, parent, projections);
            projections.push(ReferencePlaceProjection::Field(field.clone()));
        }
        ScalarPlace::Index { base, index, .. } => {
            collect_reference_projections(base, parent, projections);
            projections.push(ReferencePlaceProjection::Index(*index.clone()));
        }
    }
}

pub(super) fn reference_places_overlap(left: &ReferencePlaceId, right: &ReferencePlaceId) -> bool {
    if left.binding != right.binding {
        return false;
    }
    for (left, right) in left.projections.iter().zip(&right.projections) {
        match (left, right) {
            (
                ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(left)),
                ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(right)),
            ) if left != right => return false,
            (
                ReferencePlaceProjection::Index(ScalarExpression::Integer { value: left, .. }),
                ReferencePlaceProjection::Index(ScalarExpression::Integer { value: right, .. }),
            ) if left != right => return false,
            _ => {}
        }
    }
    true
}

struct ValidatedCallResolver<'a> {
    source: &'a SourceIdentity,
    function_span: ByteSpan,
    module_callables: &'a BTreeMap<(Option<String>, String), (SourceIdentity, ByteSpan)>,
    extern_callables: &'a BTreeMap<(String, String), (SourceIdentity, ByteSpan)>,
    modules: &'a [ScalarModule],
    sites: Vec<ResolvedCallSite>,
}

impl ValidatedCallResolver<'_> {
    fn expression(
        &mut self,
        expression: &ScalarExpression,
        names: &BTreeMap<String, ReferenceBindingId>,
        types: &BTreeMap<String, ScalarType>,
    ) -> Result<(), ReferenceAnalysisError> {
        match expression {
            ScalarExpression::Call {
                receiver,
                receiver_span,
                name,
                arguments,
                overload_selection,
                span,
                ..
            } => {
                let target = if receiver.as_deref() == Some("core") {
                    CoreOperationId::from_name(name).map(ResolvedCallTarget::Core)
                } else if receiver.is_none() {
                    names
                        .get(name)
                        .cloned()
                        .map(ResolvedCallTarget::LocalCallable)
                        .or_else(|| {
                            self.module_callables.get(&(None, name.clone())).map(
                                |(source, declaration_span)| ResolvedCallTarget::ModuleCallable {
                                    source: source.clone(),
                                    declaration_span: *declaration_span,
                                    concrete: match overload_selection {
                                        Some(selection) => {
                                            ConcreteCallSelection::Overload(selection.clone())
                                        }
                                        None => ConcreteCallSelection::Declaration,
                                    },
                                },
                            )
                        })
                } else if let Some(receiver_type) =
                    receiver.as_ref().and_then(|binding| types.get(binding))
                {
                    let ScalarType::Struct(id) = receiver_type else {
                        return Err(ReferenceAnalysisError::MissingContext {
                            source_span: SourceSpan::new(self.source.clone(), *span),
                            reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                        });
                    };
                    let field = self
                        .modules
                        .iter()
                        .find_map(|module| {
                            module.structs.iter().find(|structure| structure.id == *id)
                        })
                        .and_then(|structure| {
                            resolved_struct_field(
                                receiver_type,
                                name,
                                std::slice::from_ref(structure),
                            )
                        })
                        .expect("validated callable field");
                    let binding_name = receiver.as_ref().expect("typed field receiver");
                    let binding = names.get(binding_name).expect("validated receiver binding");
                    let receiver_span = receiver_span.expect("validated receiver source span");
                    Some(ResolvedCallTarget::CallableField {
                        receiver_place: ReferencePlaceId {
                            source: self.source.clone(),
                            function_span: self.function_span,
                            declaration_span: binding.declaration_span,
                            block_span: binding.block_span,
                            binding: binding.clone(),
                            projections: Vec::new(),
                            place: ScalarPlace::Name {
                                name: binding_name.clone(),
                                span: receiver_span,
                            },
                        },
                        field: field.id.clone(),
                    })
                } else {
                    self.module_callables
                        .get(&(receiver.clone(), name.clone()))
                        .map(
                            |(source, declaration_span)| ResolvedCallTarget::ModuleCallable {
                                source: source.clone(),
                                declaration_span: *declaration_span,
                                concrete: match overload_selection {
                                    Some(selection) => {
                                        ConcreteCallSelection::Overload(selection.clone())
                                    }
                                    None => ConcreteCallSelection::Declaration,
                                },
                            },
                        )
                        .or_else(|| {
                            self.extern_callables
                                .get(&(receiver.clone().expect("qualified call"), name.clone()))
                                .map(|(source, declaration_span)| {
                                    ResolvedCallTarget::ExternCallable {
                                        source: source.clone(),
                                        declaration_span: *declaration_span,
                                    }
                                })
                        })
                };
                let target = target.ok_or_else(|| ReferenceAnalysisError::MissingContext {
                    source_span: SourceSpan::new(self.source.clone(), *span),
                    reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                })?;
                let site = ResolvedCallSite {
                    source_span: SourceSpan::new(self.source.clone(), *span),
                    target,
                };
                if let Some(existing) = self
                    .sites
                    .iter()
                    .find(|existing| existing.source_span == site.source_span)
                {
                    if existing.target != site.target {
                        return Err(ReferenceAnalysisError::MissingContext {
                            source_span: site.source_span,
                            reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                        });
                    }
                } else {
                    self.sites.push(site);
                }
                for argument in arguments {
                    self.expression(argument, names, types)?;
                }
            }
            ScalarExpression::Binary { left, right, .. } => {
                self.expression(left, names, types)?;
                self.expression(right, names, types)?;
            }
            ScalarExpression::Unary { operand, .. } => self.expression(operand, names, types)?,
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(condition, names, types)?;
                self.block(then_branch, names, types)?;
                self.block(else_branch, names, types)?;
            }
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                self.expression(condition, names, types)?;
                self.block(then_branch, names, types)?;
            }
            ScalarExpression::Block(block) => self.block(block, names, types)?,
            ScalarExpression::RawAddress { place, .. }
            | ScalarExpression::CheckedAddress { place, .. }
            | ScalarExpression::Dereference { place, .. }
            | ScalarExpression::IndexedRead { place, .. } => self.place(place, names, types)?,
            ScalarExpression::StructLiteral { fields, .. } => {
                for field in fields {
                    self.expression(&field.value, names, types)?;
                }
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for element in elements {
                    self.expression(element, names, types)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn place(
        &mut self,
        place: &ScalarPlace,
        names: &BTreeMap<String, ReferenceBindingId>,
        types: &BTreeMap<String, ScalarType>,
    ) -> Result<(), ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { .. } => {}
            ScalarPlace::Field { base, .. } => self.place(base, names, types)?,
            ScalarPlace::Index { base, index, .. } => {
                self.place(base, names, types)?;
                self.expression(index, names, types)?;
            }
            ScalarPlace::Dereference { pointer, .. } => self.expression(pointer, names, types)?,
        }
        Ok(())
    }

    fn block(
        &mut self,
        block: &ScalarBlock,
        outer: &BTreeMap<String, ReferenceBindingId>,
        outer_types: &BTreeMap<String, ScalarType>,
    ) -> Result<(), ReferenceAnalysisError> {
        let mut names = outer.clone();
        let mut types = outer_types.clone();
        for item in &block.items {
            match item {
                ScalarBlockItem::LocalBinding(binding) => {
                    self.expression(&binding.value, &names, &types)?;
                    if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions {
                        for output in binding.output_values.iter().skip(1) {
                            self.expression(&output.value, &names, &types)?;
                        }
                    }
                    for receiver in &binding.receivers {
                        types.insert(receiver.name.clone(), receiver.ty.clone());
                        names.insert(
                            receiver.name.clone(),
                            ReferenceBindingId {
                                source: self.source.clone(),
                                function_span: self.function_span,
                                declaration_span: receiver.name_span,
                                block_span: block.span,
                                kind: ReferenceBindingKind::Declared,
                            },
                        );
                    }
                }
                ScalarBlockItem::Expression(expression) => {
                    self.expression(expression, &names, &types)?
                }
                ScalarBlockItem::Assignment(assignment) => {
                    for value in &assignment.values {
                        self.expression(value, &names, &types)?;
                    }
                    for target in &assignment.targets {
                        self.place(&target.place, &names, &types)?;
                    }
                    for target in &assignment.targets {
                        if let ScalarPlace::Name { name, .. } = &target.place {
                            if !names.contains_key(name) {
                                if let ScalarExpression::Name { name: source, .. } =
                                    &assignment.value
                                {
                                    if let Some(ty) = types.get(source).cloned() {
                                        types.insert(name.clone(), ty);
                                    }
                                }
                                names.insert(
                                    name.clone(),
                                    ReferenceBindingId {
                                        source: self.source.clone(),
                                        function_span: self.function_span,
                                        declaration_span: target.target_span,
                                        block_span: block.span,
                                        kind: ReferenceBindingKind::Declared,
                                    },
                                );
                            }
                        }
                    }
                }
                ScalarBlockItem::While(while_expression) => {
                    self.expression(&while_expression.condition, &names, &types)?;
                    self.block(&while_expression.body, &names, &types)?;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn resolve_module_call_sites(
    source: &SourceIdentity,
    items: &[ScalarItem],
    modules: &[ScalarModule],
) -> Result<Vec<ResolvedCallSite>, ReferenceAnalysisError> {
    let current = modules.iter().find(|module| &module.source == source);
    let mut module_callables = BTreeMap::new();
    let mut extern_callables = BTreeMap::new();
    for module in modules {
        let receiver = if &module.source == source {
            None
        } else {
            current.and_then(|current| {
                current
                    .namespace_bindings
                    .iter()
                    .find(|binding| binding.target == module.source)
                    .map(|binding| binding.binding.clone())
            })
        };
        if &module.source != source && receiver.is_none() {
            continue;
        }
        for item in &module.items {
            match item {
                ScalarItem::Function(function) => {
                    module_callables.insert(
                        (receiver.clone(), function.name.clone()),
                        (module.source.clone(), function.name_span),
                    );
                }
                ScalarItem::Binding(binding)
                    if matches!(binding.declared_type, ScalarType::Callable { .. }) =>
                {
                    module_callables.insert(
                        (receiver.clone(), binding.name.clone()),
                        (module.source.clone(), binding.name_span),
                    );
                }
                ScalarItem::Extern(extern_decl) if &module.source == source => {
                    for function in &extern_decl.functions {
                        extern_callables.insert(
                            (extern_decl.binding.clone(), function.name.clone()),
                            (module.source.clone(), function.name_span),
                        );
                    }
                }
                _ => {}
            }
        }
    }
    let mut sites = Vec::new();
    for item in items {
        if let ScalarItem::Function(function) = item {
            resolve_function_call_sites(
                source,
                function,
                &module_callables,
                &extern_callables,
                modules,
                &mut sites,
            )?;
        }
    }
    Ok(sites)
}

fn resolve_function_call_sites(
    source: &SourceIdentity,
    function: &ScalarFunction,
    module_callables: &BTreeMap<(Option<String>, String), (SourceIdentity, ByteSpan)>,
    extern_callables: &BTreeMap<(String, String), (SourceIdentity, ByteSpan)>,
    modules: &[ScalarModule],
    sites: &mut Vec<ResolvedCallSite>,
) -> Result<(), ReferenceAnalysisError> {
    for arm in &function.overload_arms {
        resolve_function_call_sites(
            source,
            arm,
            module_callables,
            extern_callables,
            modules,
            sites,
        )?;
    }
    let mut resolver = ValidatedCallResolver {
        source,
        function_span: function.span,
        module_callables,
        extern_callables,
        modules,
        sites: Vec::new(),
    };
    let mut parameter_names = BTreeMap::new();
    let mut parameter_types = BTreeMap::new();
    let ScalarType::Callable { parameters, .. } = &function.signature else {
        unreachable!("validated function signature")
    };
    for (index, (name, span)) in function
        .parameters
        .iter()
        .zip(&function.parameter_spans)
        .enumerate()
    {
        parameter_types.insert(name.clone(), parameters[index].clone());
        parameter_names.insert(
            name.clone(),
            ReferenceBindingId {
                source: source.clone(),
                function_span: function.span,
                declaration_span: *span,
                block_span: function.body.span,
                kind: ReferenceBindingKind::Declared,
            },
        );
    }
    resolver.block(&function.body, &parameter_names, &parameter_types)?;
    sites.extend(resolver.sites);
    Ok(())
}

pub(super) fn record_module_reference_origins(
    source: &SourceIdentity,
    items: &mut [ScalarItem],
    call_sites: &[ResolvedCallSite],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
) -> Vec<ReferenceAnalysisError> {
    let module_names = items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) => Some(binding.name.clone()),
            ScalarItem::Function(function) => Some(function.name.clone()),
            ScalarItem::Namespace(namespace) => Some(namespace.binding.clone()),
            ScalarItem::Extern(extern_decl) => Some(extern_decl.binding.clone()),
            ScalarItem::Executable(_) => None,
        })
        .collect::<HashSet<_>>();
    let mut errors = Vec::new();
    for item in items {
        if let ScalarItem::Function(function) = item {
            errors.extend(record_function_reference_origins(
                source,
                function,
                &module_names,
                call_sites,
                structs,
                enums,
                assignment_outputs,
            ));
        }
    }
    errors
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ReferenceCallableBody {
    Declaration,
    Overload(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReferenceOutputContract {
    source: SourceIdentity,
    declaration_span: ByteSpan,
    body: ReferenceCallableBody,
    output: usize,
    origins: Vec<ReferenceOrigin>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConcreteReferenceOutputContract {
    source: SourceIdentity,
    declaration_span: ByteSpan,
    selection: ConcreteCallSelection,
    type_arguments: Vec<ScalarType>,
    output: usize,
    origins: Vec<ReferenceOrigin>,
}

fn collect_concrete_output_contracts(
    function: &ScalarFunction,
    templates: &[ReferenceOutputContract],
    concrete: &mut Vec<ConcreteReferenceOutputContract>,
) {
    for arm in &function.overload_arms {
        collect_concrete_output_contracts(arm, templates, concrete);
    }
    for point in &function.reference_cfg.initial_points {
        let CfgPointKind::Call {
            expression: ScalarExpression::Call { type_arguments, .. },
            output,
            target:
                ResolvedCallTarget::ModuleCallable {
                    source,
                    declaration_span,
                    concrete: selection,
                },
            ..
        } = &point.kind
        else {
            continue;
        };
        let body = match selection {
            ConcreteCallSelection::Declaration => ReferenceCallableBody::Declaration,
            ConcreteCallSelection::Overload(arm) => ReferenceCallableBody::Overload(arm.arm_index),
        };
        let Some(template) = templates.iter().find(|template| {
            template.source == *source
                && template.declaration_span == *declaration_span
                && template.body == body
                && template.output == *output
        }) else {
            continue;
        };
        let contract = ConcreteReferenceOutputContract {
            source: source.clone(),
            declaration_span: *declaration_span,
            selection: selection.clone(),
            type_arguments: type_arguments
                .iter()
                .map(|argument| argument.ty.clone())
                .collect(),
            output: *output,
            origins: template.origins.clone(),
        };
        if !concrete.contains(&contract) {
            concrete.push(contract);
        }
    }
}

fn collect_function_output_contracts(
    source: &SourceIdentity,
    function: &ScalarFunction,
    declaration_span: ByteSpan,
    body: ReferenceCallableBody,
    contracts: &mut Vec<ReferenceOutputContract>,
) {
    for point in &function.reference_cfg.points {
        let CfgPointKind::ReturnOutput { output, .. } = point.kind else {
            continue;
        };
        let Some(origins) = &point.possible_origins else {
            continue;
        };
        if !matches!(&function.signature, ScalarType::Callable { outputs, .. } if matches!(outputs.outputs[output].ty, ScalarType::CheckedReference { .. }))
        {
            continue;
        }
        contracts.push(ReferenceOutputContract {
            source: source.clone(),
            declaration_span,
            body: body.clone(),
            output,
            origins: origins
                .iter()
                .map(|id| function.reference_cfg.facts[id.0].clone())
                .collect(),
        });
    }
    for (index, arm) in function.overload_arms.iter().enumerate() {
        collect_function_output_contracts(
            source,
            arm,
            declaration_span,
            ReferenceCallableBody::Overload(index),
            contracts,
        );
    }
}

fn transfer_function_output_contracts(
    function: &mut ScalarFunction,
    contracts: &[ConcreteReferenceOutputContract],
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for arm in &mut function.overload_arms {
        errors.extend(transfer_function_output_contracts(arm, contracts));
    }
    if !function.reference_cfg.initial_points.is_empty() {
        function.reference_cfg.points = function.reference_cfg.initial_points.clone();
        errors.extend(transfer_reference_facts(
            &mut function.reference_cfg,
            &BTreeMap::new(),
            contracts,
        ));
    }
    errors
}

pub(super) fn resolve_reference_output_contracts(
    modules: &mut [ScalarModule],
) -> Vec<ReferenceAnalysisError> {
    let mut contracts = Vec::new();
    loop {
        let mut concrete = Vec::new();
        for module in modules.iter() {
            for item in &module.items {
                if let ScalarItem::Function(function) = item {
                    collect_concrete_output_contracts(function, &contracts, &mut concrete);
                }
            }
        }
        let mut errors = Vec::new();
        for module in modules.iter_mut() {
            for item in &mut module.items {
                if let ScalarItem::Function(function) = item {
                    errors.extend(transfer_function_output_contracts(function, &concrete));
                }
            }
        }
        let mut next = Vec::new();
        for module in modules.iter() {
            for item in &module.items {
                if let ScalarItem::Function(function) = item {
                    collect_function_output_contracts(
                        &module.source,
                        function,
                        function.name_span,
                        ReferenceCallableBody::Declaration,
                        &mut next,
                    );
                }
            }
        }
        if next == contracts {
            return errors;
        }
        contracts = next;
    }
}

pub(super) fn record_specialized_copy_policies(
    items: &mut [ScalarItem],
    modules: &[ScalarModule],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for item in items {
        if let ScalarItem::Function(function) = item {
            errors.extend(record_function_specialized_copy_policies(
                function, modules, structs, enums,
            ));
        }
    }
    errors
}

fn record_function_specialized_copy_policies(
    function: &mut ScalarFunction,
    modules: &[ScalarModule],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for arm in &mut function.overload_arms {
        errors.extend(record_function_specialized_copy_policies(
            arm, modules, structs, enums,
        ));
    }
    let generic_parameters = function
        .generic_parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect::<BTreeSet<_>>();
    for point in &function.reference_cfg.points {
        let CfgPointKind::Call {
            expression: ScalarExpression::Call { type_arguments, .. },
            output: 0,
            target:
                ResolvedCallTarget::ModuleCallable {
                    source,
                    declaration_span,
                    concrete,
                },
            ..
        } = &point.kind
        else {
            continue;
        };
        let error = || ReferenceAnalysisError::MissingContext {
            source_span: point.source_span.clone(),
            reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
        };
        let Some(declaration) = modules
            .iter()
            .find(|module| module.source == *source)
            .and_then(|module| {
                module.items.iter().find_map(|item| match item {
                    ScalarItem::Function(candidate) if candidate.name_span == *declaration_span => {
                        Some(candidate)
                    }
                    _ => None,
                })
            })
        else {
            errors.push(error());
            continue;
        };
        let template = match concrete {
            ConcreteCallSelection::Overload(selection) => {
                declaration.overload_arms.get(selection.arm_index)
            }
            ConcreteCallSelection::Declaration => Some(declaration),
        };
        let Some(template) = template else {
            errors.push(error());
            continue;
        };
        if template.generic_parameters.is_empty() {
            continue;
        }
        let substitutions = match concrete {
            ConcreteCallSelection::Overload(selection) => selection.substitutions.clone(),
            ConcreteCallSelection::Declaration => {
                if type_arguments.len() != template.generic_parameters.len() {
                    errors.push(error());
                    continue;
                }
                template
                    .generic_parameters
                    .iter()
                    .zip(type_arguments)
                    .map(|(parameter, argument)| (parameter.name.clone(), argument.ty.clone()))
                    .collect()
            }
        };
        if template
            .generic_parameters
            .iter()
            .any(|parameter| !substitutions.contains_key(&parameter.name))
        {
            errors.push(error());
            continue;
        }
        let mut deferred = BTreeMap::new();
        for template_point in &template.reference_cfg.points {
            let ty = match &template_point.kind {
                CfgPointKind::DeferredMove { ty, .. }
                | CfgPointKind::DeferredMoveThrough { ty, .. }
                | CfgPointKind::DeferredArrayElement { ty, .. } => ty,
                _ => continue,
            };
            let concrete_ty = substitute_type(ty, &substitutions);
            match copy_policy(&concrete_ty, structs, enums, &generic_parameters) {
                Ok(CopyPolicy::Move)
                    if matches!(
                        template_point.kind,
                        CfgPointKind::DeferredArrayElement { .. }
                    ) =>
                {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MoveFromArrayElement,
                    })
                }
                Ok(policy) => {
                    deferred.insert(template_point.id.0, policy);
                    function
                        .reference_cfg
                        .specialized_policies
                        .push(SpecializedCopyPolicy {
                            call_point: point.id,
                            template_source: source.clone(),
                            template_function_span: template.span,
                            template_point: template_point.id,
                            ty: concrete_ty,
                            policy,
                        })
                }
                Err(_) => errors.push(error()),
            }
        }
        if !deferred.is_empty() {
            let mut concrete_cfg = template.reference_cfg.clone();
            concrete_cfg.points = concrete_cfg.initial_points.clone();
            let transferred = transfer_reference_facts(&mut concrete_cfg, &deferred, &[]);
            errors.extend(transferred.into_iter().map(|error| {
                ReferenceAnalysisError::MissingContext {
                    source_span: point.source_span.clone(),
                    reason: error.reason(),
                }
            }));
        }
    }
    errors
}

pub(super) fn record_function_reference_origins(
    source: &SourceIdentity,
    function: &mut ScalarFunction,
    module_names: &HashSet<String>,
    call_sites: &[ResolvedCallSite],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for arm in &mut function.overload_arms {
        errors.extend(record_function_reference_origins(
            source,
            arm,
            module_names,
            call_sites,
            structs,
            enums,
            assignment_outputs,
        ));
    }
    let mut builder = ReferenceFlowBuilder {
        source: source.clone(),
        function_span: function.span,
        names: BTreeMap::new(),
        module_names: module_names.clone(),
        call_sites: call_sites.to_vec(),
        origins: HashMap::new(),
        checked: HashSet::new(),
        binding_types: HashMap::new(),
        structs: structs.to_vec(),
        enums: enums.to_vec(),
        generic_parameters: function
            .generic_parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect(),
        assignment_outputs: assignment_outputs.clone(),
        dereference_points: HashMap::new(),
        cfg: ScalarReferenceCfg {
            facts: Vec::new(),
            points: Vec::new(),
            initial_points: Vec::new(),
            allocations: BTreeSet::new(),
            specialized_policies: Vec::new(),
        },
        next_scope: 0,
        cursor: None,
    };
    builder.point(
        CfgPointKind::Entry,
        function.span,
        ReferenceScopeId(0),
        BTreeSet::new(),
    );
    for (name, span) in function.parameters.iter().zip(&function.parameter_spans) {
        builder.names.insert(
            name.clone(),
            ReferenceBindingId {
                source: source.clone(),
                function_span: function.span,
                declaration_span: *span,
                block_span: function.body.span,
                kind: ReferenceBindingKind::Declared,
            },
        );
    }
    if let ScalarType::Callable { parameters, .. } = &function.signature {
        for (index, ((name, span), ty)) in function
            .parameters
            .iter()
            .zip(&function.parameter_spans)
            .zip(parameters)
            .enumerate()
        {
            builder
                .binding_types
                .insert(builder.names[name].clone(), ty.clone());
            if let ScalarType::CheckedReference { mutability, .. } = ty {
                let binding = builder.names[name].clone();
                let loan = ReferenceLoanId {
                    mode: *mutability,
                    origin_place: ReferencePlaceId {
                        source: source.clone(),
                        function_span: function.span,
                        declaration_span: *span,
                        block_span: function.body.span,
                        binding: binding.clone(),
                        projections: Vec::new(),
                        place: ScalarPlace::Name {
                            name: name.clone(),
                            span: *span,
                        },
                    },
                    creation_span: *span,
                    parent: None,
                };
                let id = builder.intern(ReferenceOrigin::BorrowedFrom {
                    parameter: index,
                    loan,
                });
                builder.checked.insert(binding.clone());
                builder
                    .origins
                    .insert(binding.clone(), BTreeSet::from([id]));
                builder.point(
                    CfgPointKind::Bind { binding },
                    *span,
                    ReferenceScopeId(0),
                    BTreeSet::from([id]),
                );
            }
        }
    }
    match builder.block(&function.body) {
        Err(error) => errors.push(error),
        Ok(result) => {
            if let ScalarType::Callable { outputs, .. } = &function.signature {
                if matches!(
                    outputs.outputs.first(),
                    Some(ScalarOutput {
                        ty: ScalarType::CheckedReference { .. },
                        ..
                    })
                ) && result.is_none()
                    && !function.body.final_output_values.is_empty()
                {
                    let span = function.body.final_output_values[0].span;
                    errors.push(builder.missing_context(
                        span,
                        ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                    ));
                }
            }
        }
    }
    if errors.is_empty() && !builder.cfg.points.is_empty() {
        builder.cfg.initial_points = builder.cfg.points.clone();
        let _ = transfer_reference_facts(&mut builder.cfg, &BTreeMap::new(), &[]);
    }
    function.reference_cfg = builder.cfg;
    errors
}

pub(super) fn expand_conditional_final_outputs(block: &mut ScalarBlock, outputs: &[ScalarOutput]) {
    let output_count = outputs.len();
    if block.final_output_values.len() != 1 || output_count < 2 {
        return;
    }
    let ScalarExpression::If {
        condition,
        then_branch,
        else_branch,
        ..
    } = &block.final_output_values[0].value
    else {
        return;
    };
    let mut then_branch = then_branch.clone();
    let mut else_branch = else_branch.clone();
    expand_conditional_final_outputs(&mut then_branch, outputs);
    expand_conditional_final_outputs(&mut else_branch, outputs);
    if then_branch.final_output_values.len() != output_count
        || else_branch.final_output_values.len() != output_count
    {
        return;
    }
    let condition = condition.clone();
    let conditional_span = span_of(&block.final_output_values[0].value);
    block.final_output_values = (0..output_count)
        .map(|position| {
            let value = ScalarExpression::If {
                condition: condition.clone(),
                then_branch: select_final_output(&then_branch, position, &outputs[position].ty),
                else_branch: select_final_output(&else_branch, position, &outputs[position].ty),
                span: conditional_span,
            };
            ScalarOutputValue {
                position,
                ty: ScalarType::Error,
                span: span_of(&value),
                value,
            }
        })
        .collect();
    let final_items = block
        .final_output_values
        .iter()
        .map(|output| ScalarBlockItem::Expression(output.value.clone()));
    let final_start = block.items.len() - 1;
    block.items.truncate(final_start);
    block.terminated_items.truncate(final_start);
    block.items.extend(final_items);
    block
        .terminated_items
        .extend((0..output_count).map(|_| false));
    block.expressions = block
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
}

fn select_final_output(block: &ScalarBlock, position: usize, ty: &ScalarType) -> ScalarBlock {
    let mut selected = block.clone();
    let final_start = selected.items.len() - selected.final_output_values.len();
    selected.items.truncate(final_start);
    selected.terminated_items.truncate(final_start);
    selected.final_output_values = vec![block.final_output_values[position].clone()];
    selected.final_output_values[0].ty = ty.clone();
    selected.items.push(ScalarBlockItem::Expression(
        selected.final_output_values[0].value.clone(),
    ));
    selected.terminated_items.push(false);
    selected.expressions = selected
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
    selected
}

use super::reborrow::{self, AccessAuthority};
use super::typecheck::expression_type_in_module;
use super::*;

#[path = "allocation_address.rs"]
mod allocation_address;

#[path = "assignment_kernel.rs"]
mod assignment_kernel;

#[cfg(test)]
#[path = "evaluated_aggregate_source_tests.rs"]
mod evaluated_aggregate_source_tests;

#[path = "assignment_saved_flow.rs"]
mod assignment_saved_flow;

#[path = "assignment_frozen_flow.rs"]
mod assignment_frozen_flow;

use assignment_frozen_flow::{FrozenAssignmentAccess, FrozenAssignmentDestination};

#[path = "output_sequence.rs"]
mod output_sequence;

#[path = "binding_holder.rs"]
mod binding_holder;
pub(super) mod parameter_input;
mod read_choice;
pub(super) mod name_resolution;
#[path = "evaluated_aggregate_receiving.rs"]
mod evaluated_aggregate_receiving;

#[path = "conditional_output_production.rs"]
pub(super) mod conditional_output_production;

#[path = "source_epoch.rs"]
pub(super) mod source_epoch;

#[cfg(test)]
pub(super) use output_sequence::assert_call_binding_channels;
#[cfg(test)]
pub(super) use output_sequence::validate_binding_inputs;
pub(super) use output_sequence::binding_disposition;
#[cfg(test)]
pub(super) use output_sequence::assert_control_binding_channels;
#[cfg(test)]
pub(super) use output_sequence::assert_control_binding_rejection;

#[path = "reference_state.rs"]
mod reference_state;

#[cfg(test)]
#[path = "reference_state_tests.rs"]
mod reference_state_tests;

#[cfg(test)]
#[path = "pointer_type_tests.rs"]
mod pointer_type_tests;

#[cfg(test)]
#[path = "allocation_address_tests.rs"]
mod allocation_address_tests;
#[cfg(test)]
#[path = "reference_test_observations.rs"]
pub(super) mod test_observations;
#[cfg(test)]
use test_observations::*;

pub(super) fn record_program_reference_origins(
    program: &mut ScalarProgram,
    call_sites: &[ResolvedCallSite],
    captures: &[(ScalarCaptureOccurrence, CfgPointKind)],
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    let mut errors = record_module_reference_origins(
        &program.source,
        &mut program.items,
        &mut program.top_level_reference_cfg,
        call_sites,
        captures,
        &program.structs,
        &program.enums,
        &program.resolved_assignment_outputs.borrow(),
        &program.resolved_literals.borrow(),
        &assignment::SourceValueTypeFacts {
            assignments: program.resolved_assignment_types.borrow().clone(),
            expressions: program.resolved_expression_outputs.borrow().clone(),
        },
        #[cfg(test)]
        trace,
    );
    if errors.is_empty() {
        let mut modules = vec![ScalarModule::from_program(program.clone(), Vec::new())];
        errors.extend(resolve_reference_output_contracts(
            &mut modules,
            #[cfg(test)]
            trace,
        ));
        let module = modules.remove(0);
        program.items = module.items;
        program.top_level_reference_cfg = module.top_level_reference_cfg;
        program.terminal_owners = program_terminal_owners(
            &program.source,
            &program.items,
            &program.top_level_reference_cfg,
        );
    }
    errors
}

pub(super) struct ReferenceFlowBuilder {
    rebind_association_count: usize,
    rebind_associations: BTreeMap<usize, ReferenceRebindAssociation>,
    pub(super) source: SourceIdentity,
    function_span: ByteSpan,
    assignment_context_span: ByteSpan,
    pub(super) names: BTreeMap<String, ReferenceBindingId>,
    global_bindings: BTreeMap<String, (ReferenceBindingId, ScalarType)>,
    module_names: HashSet<String>,
    call_sites: Vec<ResolvedCallSite>,
    pub(super) origins: HashMap<ReferenceBindingId, BTreeSet<ReferenceOriginId>>,
    pub(super) checked: HashSet<ReferenceBindingId>,
    pub(super) binding_types: HashMap<ReferenceBindingId, ScalarType>,
    pub(super) structs: Vec<ScalarStruct>,
    pub(super) enums: Vec<ScalarEnum>,
    pub(super) generic_scope: ProducerCfgOwnerId,
    pub(super) generic_declarations: Vec<ScalarGenericParameter>,
    pub(super) generic_parameters: BTreeSet<String>,
    assignment_outputs: HashMap<ResolvedAssignmentOutputId, ScalarType>,
    pub(super) value_types: assignment::SourceValueTypeFacts,
    literal_signatures: Vec<closure::ResolvedLiteralSignature>,
    pub(super) dereference_points: HashMap<ByteSpan, CfgPointId>,
    pub(super) index_points: HashMap<ByteSpan, CfgPointId>,
    array_elements: Box<HashMap<CfgPointId, Vec<CfgPointId>>>,
    pub(super) cfg: ScalarReferenceCfg,
    next_scope: usize,
    cursor: Option<CfgPointId>,
}

#[cfg(test)]
impl ReferenceFlowBuilder {
    pub(super) fn test_builder(source: &SourceIdentity, function: &ScalarFunction) -> Self {
        Self {
            rebind_association_count: 0,
            rebind_associations: BTreeMap::new(),
            source: source.clone(),
            function_span: function.span,
            assignment_context_span: function.span,
            names: BTreeMap::new(),
            global_bindings: BTreeMap::new(),
            module_names: HashSet::new(),
            call_sites: Vec::new(),
            origins: HashMap::new(),
            checked: HashSet::new(),
            binding_types: HashMap::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            generic_scope: function.reference_cfg.producer_owner.clone(),
            generic_declarations: function.generic_parameters.clone(),
            generic_parameters: BTreeSet::new(),
            assignment_outputs: HashMap::new(),
            value_types: assignment::SourceValueTypeFacts {
                assignments: Vec::new(),
                expressions: Vec::new(),
            },
            literal_signatures: Vec::new(),
            dereference_points: HashMap::new(),
            index_points: HashMap::new(),
            array_elements: Box::new(HashMap::new()),
            cfg: ScalarReferenceCfg {
                source_capture_inventory: SourceCaptureInventory::building(),
                name_resolution_context: NameResolutionContext::Building,
                module_resolver_input: source_boundary::SourceModuleResolverInputState::NotApplicable,
                source_null_constructors: Vec::new(),
                evaluated_values: EvaluatedValueTables::new(),
                borrow_evaluations: Vec::new(),
                rebind_associations: Vec::new(),
                allocation_address_evaluations: Vec::new(),
                rebind_evaluations: Vec::new(),
                checked_installs: Vec::new(),
                checked_install_evaluations: Vec::new(),
                checked_bindings: BTreeMap::new(),
                producer_owner: function.reference_cfg.producer_owner.clone(),
                environments: Vec::new(),
                environment_imports: Vec::new(),
                inputs: Vec::new(),
                capture_input_paths: Vec::new(),
                capture_input_path_facts: Vec::new(),
                producer_return_cases: Vec::new(),
                global_callable_input_paths: Vec::new(),
                global_call_exit_facts: Vec::new(),
                global_call_exit_imports: Vec::new(),
                nested_call_installs: Vec::new(),
                backing_parameters: Vec::new(),
                callable_array_parameters: Vec::new(),
                backing_length_sites: Vec::new(),
                backing_input_paths: Vec::new(),
                projected_backing_input_paths: Vec::new(),
                call_signatures: Vec::new(),
                backing_output_contracts: Vec::new(),
                literal_cfgs: Vec::new(),
                facts: Vec::new(),
                points: Vec::new(),
                while_points: Vec::new(),
                block_scope_exits: Vec::new(),
                conditional_points: Vec::new(),
                call_points: Vec::new(),
                capture_points: Vec::new(),
                read_points: Vec::new(),
                binding_points: Vec::new(),
                assignment_points: Vec::new(),
                top_level: Box::default(),
                holder_edges: Vec::new(),
                holder_exits: Vec::new(),
                initial_points: Vec::new(),
                allocations: BTreeSet::new(),
                reference_parameters: Vec::new(),
                specialized_policies: Vec::new(),
            },
            next_scope: 0,
            cursor: None,
        }
    }
}

#[derive(Debug)]
pub(super) enum ReferenceValue {
    Other,
    Checked(BTreeSet<ReferenceOriginId>),
    Uncomputed,
}

struct ReferenceBlockEvaluation {
    value: Option<ReferenceValue>,
    outputs: Vec<assignment::EvaluatedOutputEntry>,
    scope_exit: CfgPointId,
}

impl ReferenceFlowBuilder {
    fn record_backing_length_site(
        &mut self,
        binding: &ScalarBinding,
        id: &ReferenceBindingId,
        value_point: Option<CfgPointId>,
    ) -> Result<(), ReferenceAnalysisError> {
        let length = if let Some(expression) = &binding.receivers[0].allocation_length {
            RuntimeArrayLengthFact::Evaluated {
                expression: expression.clone(),
                occurrence: SourceSpan::new(self.source.clone(), expression_span(expression)),
                point: value_point.ok_or_else(|| {
                    self.missing_context(
                        binding.span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    )
                })?,
                function_span: self.function_span,
            }
        } else if let ScalarType::Array {
            length,
            length_span,
            ..
        } = &binding.declared_type
        {
            RuntimeArrayLengthFact::Fixed {
                value: *length,
                origin: SourceSpan::new(self.source.clone(), *length_span),
            }
        } else {
            return Ok(());
        };
        self.cfg
            .backing_length_sites
            .push(RuntimeBackingLengthSite {
                binding: id.clone(),
                length,
                element: match &binding.declared_type {
                    ScalarType::Array { element, .. }
                    | ScalarType::RuntimeArray { element, .. } => *element.clone(),
                    _ => {
                        return Err(self.missing_context(
                            binding.span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        ))
                    }
                },
            });
        Ok(())
    }
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
                    function_span: self.assignment_context_span,
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
        evaluated_borrow::intern(&mut self.cfg.facts, origin)
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
        let install = match &kind {
            CfgPointKind::Assign {
                target, rhs_point, ..
            } if matches!(
                self.indexed_move_type(&target.place)
                    .expect("resolved assignment type"),
                ScalarType::CheckedReference { .. }
            ) =>
            {
                Some(ReferenceCheckedInstallInput::Evaluated {
                    value_point: *rhs_point,
                })
            }
            CfgPointKind::AssignThrough {
                place, rhs_point, ..
            } if matches!(
                self.indexed_move_type(place)
                    .expect("resolved through assignment type"),
                ScalarType::CheckedReference { .. }
            ) =>
            {
                Some(ReferenceCheckedInstallInput::Evaluated {
                    value_point: *rhs_point,
                })
            }
            _ => None,
        };
        if let Some(input) = install {
            self.cfg
                .checked_installs
                .push(ReferenceCheckedInstall { point: id, input });
        }
        if let Some(previous) = self.cursor {
            self.cfg.points[previous.0].successors.push(id);
        }
        self.cfg.points.push(ReferenceFlowPoint {
            id,
            kind,
            source_span: SourceSpan::new(self.source.clone(), span),
            scope,
            possible_origins,
            possible_environments: BTreeSet::new(),
            possible_projected_environments: Vec::new(),
            possible_callee_environments: BTreeSet::new(),
            possible_backings: Vec::new(),
            possible_projected_backings: Vec::new(),
            backing_argument_paths: Vec::new(),
            global_callable_call_paths: Vec::new(),
            global_callable_output_effects: Vec::new(),
            call_import_paths: Vec::new(),
            successors: Vec::new(),
        });
        if let CfgPointKind::Call {
            expression: ScalarExpression::Call { occurrence, .. },
            output,
            ..
        } = &self.cfg.points[id.0].kind
        {
            let occurrence = occurrence.as_ref().clone();
            let output = *output;
            if output == 0 {
                self.cfg.call_points.push(ScalarCallCfgPoint {
                    occurrence,
                    evaluated_point: id,
                    projections: vec![ScalarCallOutputProjection { output, point: id }],
                });
            } else {
                let mut matches = self
                    .cfg
                    .call_points
                    .iter_mut()
                    .filter(|call| call.occurrence == occurrence)
                    .collect::<Vec<_>>();
                let [call] = matches.as_mut_slice() else {
                    panic!("projected call must have one exact evaluated occurrence");
                };
                call.projections
                    .push(ScalarCallOutputProjection { output, point: id });
            }
        }
        self.cursor = Some(id);
        id
    }

    fn uncomputed_point(&mut self, kind: CfgPointKind, span: ByteSpan, scope: ReferenceScopeId) {
        self.point_with_origins(kind, span, scope, None);
    }

    fn checked_bind_input(&mut self, point: CfgPointId, value_point: CfgPointId) {
        self.cfg.checked_installs.push(ReferenceCheckedInstall {
            point,
            input: ReferenceCheckedInstallInput::Evaluated { value_point },
        });
    }

    fn checked_storage(&mut self, binding: &ReferenceBindingId, place: ScalarPlace) {
        self.cfg.checked_bindings.insert(
            binding.clone(),
            ReferencePlaceId {
                source: binding.source.clone(),
                function_span: binding.function_span,
                declaration_span: binding.declaration_span,
                block_span: binding.block_span,
                binding: binding.clone(),
                projections: Vec::new(),
                place,
            },
        );
    }

    fn checked_global_storage(&mut self) {
        for (name, (binding, ty)) in self.global_bindings.clone() {
            if matches!(ty, ScalarType::CheckedReference { .. }) {
                self.checked_storage(
                    &binding,
                    ScalarPlace::Name {
                        name,
                        span: binding.declaration_span,
                    },
                );
            }
        }
    }

    fn binding_point(
        &mut self,
        binding: &ScalarBinding,
        output: usize,
        role: ScalarBindingPointRole,
        point: CfgPointId,
    ) -> Result<(), ReferenceAnalysisError> {
        let source = self.binding_output_source(binding, output)?;
        self.cfg.binding_points.push(ScalarBindingCfgPoint {
            occurrence: binding.occurrence.as_ref().clone(),
            output,
            role,
            point,
            source,
        });
        Ok(())
    }

    fn bind_callable_array_elements(
        &mut self,
        binding: &ScalarBinding,
        value_point: CfgPointId,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        let (ScalarType::Array { .. }, ScalarExpression::ArrayLiteral { elements, .. }) =
            (&binding.declared_type, &binding.value)
        else {
            return Ok(());
        };
        let sources = self.cfg.evaluated_values.values.iter().filter(|fact| fact.id.point == value_point).collect::<Vec<_>>();
        let [source] = sources.as_slice() else { return Err(self.missing_context(binding.span, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence)); };
        if evaluated_aggregate_receiving::members(&self.cfg, &source.id)?.is_empty() { return Ok(()); }
        let evaluated = self.array_elements.remove(&value_point).ok_or_else(|| {
            self.missing_context(
                binding.span,
                ReferenceAnalysisErrorReason::MissingResolvedBinding,
            )
        })?;
        if evaluated.len() != elements.len() {
            return Err(self.missing_context(
                binding.span,
                ReferenceAnalysisErrorReason::MissingResolvedBinding,
            ));
        }
        for (index, (element, rhs_point)) in elements.iter().zip(evaluated).enumerate() {
            let span = expression_span(element);
            let place = ScalarPlace::Index {
                base: Box::new(ScalarPlace::Name {
                    name: binding.name.clone(),
                    span: binding.name_span,
                }),
                index: Box::new(ScalarExpression::Integer {
                    value: index.into(),
                    span,
                }),
                span,
                index_span: span,
            };
            let target = self.assignment_place(&place, span)?;
            let point = self.point(
                CfgPointKind::BindArrayElement { target, rhs_point },
                span,
                scope,
                BTreeSet::new(),
            );
            self.binding_point(
                binding,
                0,
                ScalarBindingPointRole::ArrayElement(index),
                point,
            )?;
        }
        Ok(())
    }

    fn copy_evaluated_aggregate_binding(
        &mut self,
        binding: &ScalarBinding,
        target: &ReferenceBindingId,
        output: usize,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        if !matches!(binding.receivers[output].ty, ScalarType::Struct(_) | ScalarType::Enum(_) | ScalarType::Array { .. }) { return Ok(()); }
        let retained = self.binding_output_source(binding, output)?;
        if matches!(retained, ScalarBindingOutputSource::Expression { .. }) {
            return Ok(());
        }
        let source = evaluated_aggregate_receiving::source(&self.cfg, target)?;
        let fact = self.cfg.evaluated_values.values.iter().find(|fact| fact.id == source).expect("registered actual Binding source");
        let shape = self.cfg.evaluated_values.shapes.iter().find(|shape| shape.id == fact.shape).expect("registered actual source shape");
        if !matches!(shape.payload, ValuePayloadShape::Struct { .. } | ValuePayloadShape::Enum { .. } | ValuePayloadShape::FixedArray { .. })
            || evaluated_aggregate_receiving::members(&self.cfg, &source)?.is_empty() { return Ok(()); }
        let receiving_shapes = self.cfg.evaluated_values.source_binding_shapes.iter().filter(|row| row.binding == *target).collect::<Vec<_>>();
        let [receiving_shape] = receiving_shapes.as_slice() else { return Err(self.missing_context(binding.name_span, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence)); };
        if receiving_shape.shape != shape.id { return Err(self.missing_context(binding.name_span, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence)); }
        if matches!(&self.cfg.points[source.point.0].kind, CfgPointKind::Evaluate(ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. }))
            && self.cfg.evaluated_values.binding_inputs.iter().filter(|input| input.source == source
                && input.occurrence == *binding.occurrence).all(|input| input.output >= output) { return Ok(()); }
        let copy = self.point(
            CfgPointKind::CopyCallableFields {
                source: CallableFieldSource::Evaluated(source),
                target: target.clone(),
            },
            binding.name_span,
            scope,
            BTreeSet::new(),
        );
        self.binding_point(binding, output, ScalarBindingPointRole::CopyCallableFields, copy)?;
        Ok(())
    }

    fn assignment_point(
        &mut self,
        assignment: &ScalarAssignment,
        target: usize,
        role: ScalarAssignmentPointRole,
        point: CfgPointId,
    ) {
        self.cfg.assignment_points.push(ScalarAssignmentCfgPoint {
            occurrence: assignment.occurrence.as_ref().clone(),
            target,
            role,
            point,
        });
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

    pub(super) fn missing_context(
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
        if let Some(binding) = self.names.get(name) {
            return Ok(binding.clone());
        }
        if let Some((binding, _)) = self.global_bindings.get(name) {
            return Ok(binding.clone());
        }
        Err(ReferenceAnalysisError::MissingBinding {
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
            ScalarPlace::Name { name, span } => (
                if let Some(binding) = self.names.get(name) {
                    binding.clone()
                } else if let Some((binding, _)) = self.global_bindings.get(name) {
                    binding.clone()
                } else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                },
                None,
            ),
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
        for projection in &mut projections {
            if let ReferencePlaceProjection::Field(ScalarFieldReference::Unresolved {
                name,
                span,
            }) = projection
            {
                let ScalarType::Struct(structure) = self
                    .binding_types
                    .get(&binding)
                    .or_else(|| {
                        self.global_bindings
                            .values()
                            .find(|(id, _)| id == &binding)
                            .map(|(_, ty)| ty)
                    })
                    .ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?
                else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                };
                let field = self
                    .structs
                    .iter()
                    .find(|item| item.id == *structure)
                    .and_then(|item| item.fields.iter().find(|field| field.name == *name))
                    .ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                *projection = ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(
                    field.id.clone(),
                ));
            }
        }
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
                    creation_source: self.source.clone(),
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
        self.cfg.conditional_points.push(ScalarConditionalCfgPoint {
            then_block: then_branch.occurrence.as_ref().clone(),
            role: ScalarConditionalPointRole::Branch,
            point: branch,
        });
        let incoming_names = self.names.clone();
        let incoming_origins = self.origins.clone();
        let incoming_checked = self.checked.clone();
        let then_evaluation = self.block(then_branch)?;
        let then_end = then_evaluation.scope_exit;
        self.cfg.conditional_points.push(ScalarConditionalCfgPoint {
            then_block: then_branch.occurrence.as_ref().clone(),
            role: ScalarConditionalPointRole::ThenScopeExit,
            point: then_end,
        });
        let then_checked = self.checked.clone();

        self.names = incoming_names;
        self.origins = incoming_origins.clone();
        self.checked = incoming_checked.clone();
        self.cursor = Some(branch);
        let (else_value, else_end, else_outputs) = if let Some(else_branch) = else_branch {
            let result = self.block(else_branch)?;
            let exit = self.cursor.expect("else branch terminal");
            self.cfg.conditional_points.push(ScalarConditionalCfgPoint {
                then_block: then_branch.occurrence.as_ref().clone(),
                role: ScalarConditionalPointRole::ElseScopeExit,
                point: exit,
            });
            (result.value, exit, result.outputs)
        } else {
            (None, branch, Vec::new())
        };
        self.checked.extend(then_checked);
        self.origins.clear();
        self.cursor = None;
        let joined_value = then_evaluation.value.is_some() && else_value.is_some();
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
        if else_branch.is_some() {
            self.complete_control_outputs(
                SourceSpan::new(self.source.clone(), span),
                then_branch.occurrence.output_projection.clone(),
                join,
                scope,
                OutputProductionSource::Conditional {
                    branch,
                    inputs: vec![
                        ConditionalOutputInput {
                            predecessor: then_end,
                            outputs: then_evaluation.outputs.into_iter().map(|entry| entry.value).collect(),
                        },
                        ConditionalOutputInput {
                            predecessor: else_end,
                            outputs: else_outputs.into_iter().map(|entry| entry.value).collect(),
                        },
                    ],
                },
            )?;
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
        let result = self.expression_inner(expression, scope)?;
        if !matches!(
            expression,
            ScalarExpression::Assignment(_)
                | ScalarExpression::Call { .. }
                | ScalarExpression::If { .. }
                | ScalarExpression::UnitIf { .. }
                | ScalarExpression::Block(_)
        ) {
            let point = self.expression_value_point(expression, 0)?;
            self.bind_final_output_values(expression, point)?;
            name_resolution::record(self, expression, point)?;
        }
        Ok(result)
    }

    fn expression_inner(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let span = expression_span(expression);
        match expression {
            ScalarExpression::Assignment(assignment) => {
                let block_span = self
                    .cfg
                    .points
                    .iter()
                    .find(|point| {
                        point.scope == scope
                            && matches!(point.kind, CfgPointKind::ScopeEnter | CfgPointKind::Entry)
                    })
                    .expect("assignment containing scope")
                    .source_span
                    .range;
                self.assignment(assignment, block_span, scope, &mut Vec::new(), true)?;
                Ok(ReferenceValue::Uncomputed)
            }
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
            ScalarExpression::Name { name, .. } if name == "null" => {
                let id = self.intern(ReferenceOrigin::Null { span });
                let origins = BTreeSet::from([id]);
                let point = self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    origins.clone(),
                );
                let constructor = source_boundary::SourceNullConstructor {
                    origin: source_boundary::SourceConstructorOriginRef { owner: self.cfg.producer_owner.clone(), origin: id },
                    event: SourceEventId { owner: self.cfg.producer_owner.clone(), point },
                    source_span: SourceSpan::new(self.source.clone(), span), scope,
                    expression: expression.clone(), definition: ReferenceOrigin::Null { span },
                };
                let derivation = constructor.derivation();
                self.cfg.source_null_constructors.push(constructor);
                self.cfg.evaluated_values.source_origin_derivations.push(derivation);
                Ok(ReferenceValue::Checked(origins))
            }
            ScalarExpression::Name { name, .. } => {
                if self.module_names.contains(name) && !self.names.contains_key(name) {
                    if let Some((binding, ty)) = self.global_bindings.get(name).cloned() {
                        if matches!(ty, ScalarType::CheckedReference { .. }) {
                            self.checked.insert(binding.clone());
                            self.binding_types.insert(binding.clone(), ty.clone());
                            self.uncomputed_point(CfgPointKind::Read { binding }, span, scope);
                            return Ok(ReferenceValue::Uncomputed);
                        }
                        self.binding_types.insert(binding.clone(), ty.clone());
                        self.point(CfgPointKind::Read { binding }, span, scope, BTreeSet::new());
                        return Ok(ReferenceValue::Other);
                    }
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
            _ => self.composite_expression(expression, scope),
        }
    }

    fn composite_expression(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let span = expression_span(expression);
        match expression {
            ScalarExpression::FunctionLiteral {
                parameters,
                parameter_spans,
                body,
                ..
            } => {
                let captures = closure::captures(
                    &self.source,
                    expression,
                    &self.names,
                    &self.binding_types,
                    &self.structs,
                );
                let signature = self
                    .literal_signatures
                    .iter()
                    .find(|entry| entry.literal == SourceSpan::new(self.source.clone(), span))
                    .expect("validated literal signature")
                    .signature
                    .clone();
                let environment = CallableEnvironmentId {
                    literal: SourceSpan::new(self.source.clone(), span),
                    index: self.cfg.environments.len(),
                };
                self.cfg.environments.push(CallableEnvironmentFact {
                    literal: SourceSpan::new(self.source.clone(), span),
                    instance: CallableEnvironmentInstance::Capture,
                    origins: BTreeSet::new(),
                    owners: BTreeSet::new(),
                    loans: BTreeSet::new(),
                    backings: Vec::new(),
                    inputs: Vec::new(),
                    capture_paths: Vec::new(),
                });
                let point = self.point(
                    CfgPointKind::Capture {
                        literal: span,
                        environment,
                        signature,
                        captures,
                        parameters: parameters.clone(),
                        parameter_spans: parameter_spans.clone(),
                        body: body.clone(),
                    },
                    span,
                    scope,
                    BTreeSet::new(),
                );
                self.cfg.capture_points.push(ScalarCaptureCfgPoint {
                    occurrence: ScalarCaptureOccurrence::for_literal(span, body),
                    role: ScalarCapturePointRole::Capture,
                    point,
                });
                self.register_capture_input(point)?;
                Ok(ReferenceValue::Other)
            }
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
            ScalarExpression::Block(block) => {
                let evaluation = self.block(block)?;
                let result = match evaluation.value {
                Some(ReferenceValue::Checked(origins)) => {
                    self.point(
                        CfgPointKind::Evaluate(expression.clone()),
                        span,
                        scope,
                        origins.clone(),
                    );
                    ReferenceValue::Checked(origins)
                }
                Some(ReferenceValue::Uncomputed) => {
                    self.uncomputed_point(CfgPointKind::Evaluate(expression.clone()), span, scope);
                    ReferenceValue::Uncomputed
                }
                _ => {
                    self.point(
                        CfgPointKind::Evaluate(expression.clone()),
                        span,
                        scope,
                        BTreeSet::new(),
                    );
                    ReferenceValue::Other
                }
                };
                let anchor = self.cursor.expect("actual block value evaluation");
                self.complete_control_outputs(
                    SourceSpan::new(self.source.clone(), span),
                    block.occurrence.output_projection.clone(),
                    anchor,
                    scope,
                    OutputProductionSource::Block {
                        occurrence: (*block.occurrence).clone(),
                        scope_exit: evaluation.scope_exit,
                        inputs: evaluation.outputs.into_iter().map(|entry| entry.value).collect(),
                    },
                )?;
                Ok(result)
            },
            ScalarExpression::Call { arguments, .. } => {
                let target = self.call_target(expression)?;
                let identity_association = if target
                    == ResolvedCallTarget::Core(CoreOperationId::Rebind)
                    && matches!(&arguments[0], ScalarExpression::CheckedAddress { place, .. }
                        if matches!(reference_root(place), ScalarPlace::Name { .. }))
                {
                    let id = ReferenceRebindAssociationId(self.rebind_association_count);
                    self.rebind_association_count += 1;
                    Some(id)
                } else {
                    None
                };
                let mut address = None;
                let mut rebind_effect = None;
                let mut checked_release_address = None;
                let mut argument_points = Vec::new();
                let mut argument_inputs = Vec::new();
                for (position, argument) in arguments.iter().enumerate() {
                    if let Some(association) = identity_association.filter(|_| position == 0) {
                        self.allocation_identity_address(argument, association, scope)?;
                    } else {
                        self.expression(argument, scope)?;
                    }
                    let argument_point = self.expression_value_point(argument, 0)?;
                    let sequence = self.bind_final_output_values(argument, argument_point)?;
                    for entry in sequence.entries {
                        argument_points.push(entry.value.point);
                        argument_inputs.push(BoundaryArgumentInput {
                            ordinal: argument_inputs.len(),
                            source: entry.value,
                            shape: entry.shape,
                        });
                    }
                    if position == 0
                        && target == ResolvedCallTarget::Core(CoreOperationId::Free)
                        && (matches!(argument, ScalarExpression::CheckedAddress { .. })
                            || matches!(argument, ScalarExpression::Name { name, .. } if self.names.get(name).is_some_and(|binding| self.checked.contains(binding))))
                    {
                        checked_release_address = Some(argument_point);
                    }
                    if position == 0
                        && matches!(
                            target,
                            ResolvedCallTarget::Core(
                                CoreOperationId::Invalidate | CoreOperationId::Rebind
                            )
                        )
                    {
                        address = Some(argument_point);
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
                    rebind_effect = self.cursor;
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
                let result = if let Some(origins) = cast_origins {
                    self.point(call, span, scope, origins.clone());
                    ReferenceValue::Checked(origins)
                } else {
                    self.uncomputed_point(call, span, scope);
                    if let Some(association) = identity_association {
                        let ScalarExpression::Call { occurrence, .. } = expression else {
                            unreachable!()
                        };
                        let CfgPointKind::Call {
                            argument_points, ..
                        } = &self.cfg.points[self.cursor.expect("actual Rebind Call").0].kind
                        else {
                            unreachable!()
                        };
                        self.rebind_associations.insert(
                            association.0,
                            ReferenceRebindAssociation {
                                owner: self.cfg.producer_owner.clone(),
                                occurrence: occurrence.as_ref().clone(),
                                address_point: argument_points[0],
                                effect_point: rebind_effect.expect("actual Rebind effect"),
                                call_point: self.cursor.expect("actual Rebind Call"),
                                raw_point: argument_points[1],
                                length_point: argument_points[2],
                            },
                        );
                    }
                    ReferenceValue::Uncomputed
                };
                let anchor = self.cursor.expect("actual evaluated call");
                self.register_call_argument_inputs(expression, anchor, argument_inputs)?;
                self.complete_call_outputs(expression, anchor, scope)?;
                Ok(result)
            }
            ScalarExpression::Binary { .. }
            | ScalarExpression::Unary { .. }
            | ScalarExpression::Name { .. } => unreachable!("dispatched scalar expression"),
            ScalarExpression::ArrayLiteral { elements, .. } => {
                let mut evaluated = Vec::new();
                for element in elements {
                    let entry = self.evaluate_aggregate_member(element, scope)?;
                    evaluated.push(entry.value.point);
                    self.move_value(element, scope)?;
                }
                let point = self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                self.array_elements.insert(point, evaluated);
                self.register_literal_aggregate_root(expression, point)?;
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::StructLiteral { fields, .. } => {
                for field in fields {
                    self.evaluate_aggregate_member(&field.value, scope)?;
                    self.move_value(&field.value, scope)?;
                }
                let point = self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                self.register_literal_aggregate_root(expression, point)?;
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Dereference { place, .. }
            | ScalarExpression::IndexedRead { place, .. } => {
                self.checked_place_read(place, expression, span, scope)
            }
            ScalarExpression::RawAddress { place, .. } => {
                self.place_expressions(place, scope)?;
                let mut source_input = None;
                if let ScalarPlace::Name { name, .. } = reference_root(place) {
                    if self.names.contains_key(name) || self.global_bindings.contains_key(name) {
                        let candidate = self.assignment_place(place, span)?;
                        let read = self.point(
                            CfgPointKind::ReadPlace {
                                place: place.clone(),
                                pointer_point: None,
                                candidates: vec![candidate.clone()],
                                reads_origin: false,
                            },
                            span,
                            scope,
                            BTreeSet::new(),
                        );
                        source_input = Some((read, candidate));
                    }
                }
                let evaluation = self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                if let Some((read, candidate)) = source_input {
                    self.register_raw_address_input(expression, evaluation, read, candidate)?;
                } else if let ScalarPlace::Dereference { pointer, .. } = reference_root(place) {
                    let pointer_point = self.expression_value_point(pointer, 0)?;
                    let sequence = self.bind_final_output_values(pointer, pointer_point)?;
                    let [entry] = sequence.entries.as_slice() else {
                        return Err(self.missing_context(
                            span,
                            ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                        ));
                    };
                    self.register_raw_pointer_input(
                        expression,
                        evaluation,
                        entry.value.clone(),
                        place.clone(),
                    )?;
                }
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
            ScalarExpression::CheckedAddress {
                mutability, place, ..
            } => {
                let root = reference_root(place);
                if let ScalarPlace::Dereference { pointer, .. } = root {
                    self.expression(pointer, scope)?;
                    let pointer_point = self.expression_value_point(pointer, 0)?;
                    let mut indices = Vec::new();
                    self.borrow_indices(place, scope, &mut indices)?;
                    self.uncomputed_point(
                        CfgPointKind::Borrow {
                            request: ReferenceBorrowRequest::Through {
                                pointer_point,
                                place: place.clone(),
                                mode: *mutability,
                                indices,
                            },
                        },
                        span,
                        scope,
                    );
                    return Ok(ReferenceValue::Uncomputed);
                }
                let (candidate, indices) =
                    self.direct_address_candidate(place, *mutability, span, scope)?;
                let origin = candidate.origin;
                let point = self.point(
                    CfgPointKind::Borrow {
                        request: ReferenceBorrowRequest::DirectStorage {
                            place: candidate.place,
                            loan: candidate.loan,
                            origin,
                            indices,
                        },
                    },
                    span,
                    scope,
                    BTreeSet::from([origin]),
                );
                self.cfg.evaluated_values.source_origin_derivations.push(source_boundary::direct_storage_constructor::derivation(&self.cfg.producer_owner, point, origin));
                Ok(ReferenceValue::Checked(BTreeSet::from([origin])))
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
                let point = self.expression_value_point(pointer, 0)?;
                self.dereference_points.insert(*span, point);
            }
            ScalarPlace::Field { base, .. } => self.place_expressions(base, scope)?,
            ScalarPlace::Index {
                base,
                index,
                index_span,
                ..
            } => {
                self.place_expressions(base, scope)?;
                self.expression(index, scope)?;
                self.index_points
                    .insert(*index_span, self.expression_value_point(index, 0)?);
            }
        }
        Ok(())
    }

    fn select_callable_array_element(
        &mut self,
        place: &ScalarPlace,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) -> Result<Option<CfgPointId>, ReferenceAnalysisError> {
        if !matches!(place, ScalarPlace::Index { .. })
            || !matches!(self.indexed_move_type(place)?, ScalarType::Callable { .. })
        {
            return Ok(None);
        }
        let ScalarPlace::Index { index_span, .. } = place else {
            unreachable!()
        };
        let index_point = *self.index_points.get(index_span).ok_or_else(|| {
            self.missing_context(
                *index_span,
                ReferenceAnalysisErrorReason::MissingResolvedBinding,
            )
        })?;
        let target = self.assignment_place(place, span)?;
        Ok(Some(self.point(
            CfgPointKind::SelectArrayElement {
                target,
                index_point,
            },
            span,
            scope,
            BTreeSet::new(),
        )))
    }

    fn checked_place_read(
        &mut self,
        place: &ScalarPlace,
        expression: &ScalarExpression,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        if let ScalarPlace::Name { name, .. } = reference_root(place) {
            if self.module_names.contains(name)
                && !self.names.contains_key(name)
                && !self.global_bindings.contains_key(name)
            {
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
        let checked_root = match reference_root(place) {
            ScalarPlace::Dereference { pointer, .. } => match pointer.as_ref() {
                ScalarExpression::Name { name, .. } => {
                    self.checked.contains(&self.binding(name, span)?)
                }
                ScalarExpression::Call {
                    receiver: Some(receiver),
                    name,
                    type_arguments,
                    ..
                } if receiver == "core" && name == "pointer_cast" => {
                    type_arguments.first().is_some_and(|argument| {
                        matches!(argument.ty, ScalarType::CheckedReference { .. })
                    })
                }
                ScalarExpression::Call { .. } => self
                    .call_sites
                    .iter()
                    .find(|site| {
                        site.source_span
                            == SourceSpan::new(self.source.clone(), expression_span(pointer))
                    })
                    .is_some_and(|site| {
                        matches!(site.result_type, ScalarType::CheckedReference { .. })
                    }),
                _ => false,
            },
            _ => false,
        };
        let reads_checked_aggregate = checked_root
            && matches!(
                self.indexed_move_type(reference_root(place))?,
                ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
            );
        if !checked_root && matches!(reference_root(place), ScalarPlace::Dereference { .. }) {
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
            if let ScalarExpression::Dereference { occurrence, .. }
            | ScalarExpression::IndexedRead { occurrence, .. } = expression
            {
                self.cfg.read_points.push(ScalarReadCfgPoint {
                    occurrence: occurrence.as_ref().clone(),
                    function_span: self.function_span,
                    point: self.cursor.expect("evaluated ReadPlace"),
                    indices: self.evaluated_place_indices(place),
                });
            }
            Ok(ReferenceValue::Uncomputed)
        } else {
            let point = self.point(event, span, scope, BTreeSet::new());
            if let ScalarExpression::Dereference { occurrence, .. }
            | ScalarExpression::IndexedRead { occurrence, .. } = expression
            {
                self.cfg.read_points.push(ScalarReadCfgPoint {
                    occurrence: occurrence.as_ref().clone(),
                    function_span: self.function_span,
                    point,
                    indices: self.evaluated_place_indices(place),
                });
            }
            Ok(ReferenceValue::Other)
        }
    }

    fn evaluated_place_indices(&self, place: &ScalarPlace) -> Vec<ScalarIndexCfgPoint> {
        match place {
            ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => Vec::new(),
            ScalarPlace::Field { base, .. } => self.evaluated_place_indices(base),
            ScalarPlace::Index {
                base, index_span, ..
            } => {
                let mut indices = self.evaluated_place_indices(base);
                indices.push(ScalarIndexCfgPoint {
                    source_span: SourceSpan::new(self.source.clone(), *index_span),
                    point: *self
                        .index_points
                        .get(index_span)
                        .expect("evaluated place index correspondence"),
                });
                indices
            }
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

    fn borrow_indices(
        &mut self,
        place: &ScalarPlace,
        scope: ReferenceScopeId,
        inputs: &mut Vec<ReferenceBorrowIndexInput>,
    ) -> Result<usize, ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { .. } => Ok(0),
            ScalarPlace::Dereference { .. } => Ok(1),
            ScalarPlace::Field { base, .. } => Ok(self.borrow_indices(base, scope, inputs)? + 1),
            ScalarPlace::Index { base, index, .. } => {
                let projection = self.borrow_indices(base, scope, inputs)?;
                self.expression(index, scope)?;
                inputs.push(ReferenceBorrowIndexInput {
                    projection,
                    value_point: self.expression_value_point(index, 0)?,
                });
                Ok(projection + 1)
            }
        }
    }

    fn indexed_move_type(&self, place: &ScalarPlace) -> Result<ScalarType, ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { name, span } => {
                let binding = self.binding(name, *span)?;
                self.binding_types
                    .get(&binding)
                    .cloned()
                    .or_else(|| {
                        self.global_bindings
                            .values()
                            .find(|(id, _)| id == &binding)
                            .map(|(_, ty)| ty.clone())
                    })
                    .ok_or_else(|| {
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
                let source_span = SourceSpan::new(self.source.clone(), expression_span(pointer));
                let pointer_type =
                    self.value_types
                        .selected_output(&source_span, 0)
                        .map_err(|source_span| ReferenceAnalysisError::MissingContext {
                            source_span,
                            reason: ReferenceAnalysisErrorReason::InvalidPointerTypeCorrespondence,
                        })?;
                match pointer_type {
                    ScalarType::CheckedReference { inner, .. } => Ok(inner.as_ref().clone()),
                    ScalarType::RawPointer(inner) => Ok(inner.as_ref().clone()),
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
            ScalarExpression::IndexedRead { place, span, .. } => {
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
            ScalarExpression::Dereference { place, span, .. } => {
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
        let source_point = self.expression_value_point(value, 0)?;
        let sources = self
            .cfg
            .evaluated_values
            .values
            .iter()
            .filter(|fact| {
                fact.id.point == source_point
                    && fact.id.resolved.occurrence == SourceSpan::new(self.source.clone(), span)
            })
            .map(|fact| fact.id.clone())
            .collect::<Vec<_>>();
        let [source] = sources.as_slice() else {
            return Err(self.missing_context(
                span,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            ));
        };
        let source = source.clone();
        let source_place = place.clone();
        let move_point = self.point(
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
        self.cfg
            .evaluated_values
            .source_move_inputs
            .push(SourceMoveInput {
                event: SourceEventId {
                    owner: self.cfg.producer_owner.clone(),
                    point: move_point,
                },
                source,
                place: source_place,
            });
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
            let value = if binding.output_origin == ScalarBindingOutputOrigin::SingleExpression
                && matches!(binding.value, ScalarExpression::Call { .. } | ScalarExpression::If { .. } | ScalarExpression::Block(_))
            {
                &binding.value
            } else {
                &binding.output_values[position].value
            };
            if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions {
                self.expression(value, scope)?;
            }
            let retained = self.binding_output_source(binding, position)?;
            let selected_output = match &retained {
                ScalarBindingOutputSource::Direct { output, .. }
                | ScalarBindingOutputSource::Call { output, .. }
                | ScalarBindingOutputSource::Conditional { output, .. }
                | ScalarBindingOutputSource::Block { output, .. } => *output,
                ScalarBindingOutputSource::Expression { .. } => 0,
            };
            let value_point = if matches!(retained, ScalarBindingOutputSource::Direct { .. }) {
                output_sequence::retained_source_value(&self.cfg, &retained, &binding.occurrence.source_span)?.point
            } else if matches!(value, ScalarExpression::Call { .. } | ScalarExpression::If { .. } | ScalarExpression::Block(_)) {
                self.expression_value_point(value, selected_output)?
            } else {
                self.cursor.expect("evaluated additional output")
            };
            let id = ReferenceBindingId {
                source: self.source.clone(),
                function_span: self.function_span,
                declaration_span: receiver.name_span,
                block_span: block.span,
                kind: ReferenceBindingKind::Declared,
            };
            let previous = self.names.insert(receiver.name.clone(), id.clone());
            self.binding_types.insert(id.clone(), receiver.ty.clone());
            self.register_source_binding(&id, &receiver.ty)?;
            declared.push((receiver.name.clone(), id.clone(), previous));
            if matches!(receiver.ty, ScalarType::CheckedReference { .. }) {
                self.checked_storage(
                    &id,
                    ScalarPlace::Name {
                        name: receiver.name.clone(),
                        span: receiver.name_span,
                    },
                );
                self.checked.insert(id.clone());
                self.uncomputed_point(
                    CfgPointKind::Bind {
                        binding: id.clone(),
                    },
                    receiver.name_span,
                    scope,
                );
                self.checked_bind_input(self.cursor.expect("additional checked bind"), value_point);
            } else {
                self.point(
                    CfgPointKind::Bind {
                        binding: id.clone(),
                    },
                    receiver.name_span,
                    scope,
                    BTreeSet::new(),
                );
            }
            self.binding_source_point(
                binding,
                position,
                self.cursor.expect("additional receiver bind"),
            )?;
            if matches!(receiver.ty, ScalarType::RuntimeArray { .. })
                && matches!(value, ScalarExpression::Call { .. })
            {
                let call = &self.cfg.points[value_point.0];
                self.point(
                    CfgPointKind::Backing {
                        target: id.clone(),
                        transfer: RuntimeBackingTransfer::Move {
                            source: ReferenceBindingId {
                                source: self.source.clone(),
                                function_span: self.function_span,
                                declaration_span: call.source_span.range,
                                block_span: self.cfg.points[0].source_span.range,
                                kind: ReferenceBindingKind::CallOutput {
                                    point: call.id,
                                    output: selected_output,
                                },
                            },
                        },
                        materialize: true,
                    },
                    receiver.name_span,
                    scope,
                    BTreeSet::new(),
                );
            }
            self.copy_evaluated_aggregate_binding(binding, &id, position, scope)?;
        }
        Ok(())
    }

    fn block(
        &mut self,
        block: &ScalarBlock,
    ) -> Result<ReferenceBlockEvaluation, ReferenceAnalysisError> {
        let outputs = block
            .final_output_values
            .iter()
            .map(|value| ScalarOutput {
                ty: value.ty.clone(),
                span: value.span,
            })
            .collect::<Vec<_>>();
        self.block_outputs(block, &outputs)
    }

    fn block_outputs(
        &mut self,
        block: &ScalarBlock,
        receiving_outputs: &[ScalarOutput],
    ) -> Result<ReferenceBlockEvaluation, ReferenceAnalysisError> {
        let grouped = self.grouped_output_block(block)?;
        let block = match &grouped {
            Some(source) => source,
            None => block,
        };
        let scope = ReferenceScopeId(self.next_scope);
        self.next_scope += 1;
        self.point(CfgPointKind::ScopeEnter, block.span, scope, BTreeSet::new());
        let mut declared = Vec::new();
        let mut result = None;
        let mut return_values = Vec::new();
        let mut return_expressions = Vec::new();
        for (index, item) in block.items.iter().enumerate() {
            match item {
                ScalarBlockItem::LocalBinding(binding) => {
                    let value = if let Some(length) = &binding.receivers[0].allocation_length {
                        self.expression(length, scope)?
                    } else if matches!(binding.declared_type, ScalarType::RawPointer(_))
                        && matches!(&binding.value, ScalarExpression::Name { name, .. } if name == "null")
                    {
                        let point = self.point(
                            CfgPointKind::Evaluate(binding.value.clone()),
                            expression_span(&binding.value),
                            scope,
                            BTreeSet::new(),
                        );
                        self.bind_final_output_values(&binding.value, point)?;
                        ReferenceValue::Other
                    } else {
                        self.expression(&binding.value, scope)?
                    };
                    let initializer = if let Some(length) = &binding.receivers[0].allocation_length
                    {
                        length
                    } else {
                        &binding.value
                    };
                    let array_value_point = self.expression_value_point(initializer, 0)?;
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
                    self.register_source_binding(&id, &binding.declared_type)?;
                    if matches!(binding.declared_type, ScalarType::CheckedReference { .. }) {
                        self.checked_storage(
                            &id,
                            ScalarPlace::Name {
                                name: binding.name.clone(),
                                span: binding.name_span,
                            },
                        );
                    }
                    self.record_backing_length_site(binding, &id, Some(array_value_point))?;
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
                            self.checked_bind_input(
                                self.cursor.expect("primary checked bind"),
                                array_value_point,
                            );
                            self.binding_source_point(
                                binding,
                                0,
                                self.cursor.expect("primary checked bind"),
                            )?;
                            let old = self.names.insert(binding.name.clone(), id.clone());
                            declared.push((binding.name.clone(), id, old));
                            self.additional_receivers(binding, block, scope, &mut declared)?;
                            self.validate_call_receivers(binding)?;
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
                        if matches!(binding.declared_type, ScalarType::CheckedReference { .. }) {
                            self.checked_bind_input(
                                self.cursor.expect("primary checked bind"),
                                array_value_point,
                            );
                        }
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
                    self.binding_source_point(binding, 0, self.cursor.expect("primary bind"))?;
                    if matches!(&binding.declared_type, ScalarType::Array { .. })
                        && matches!(binding.value, ScalarExpression::ArrayLiteral { .. })
                    {
                        self.bind_callable_array_elements(binding, array_value_point, scope)?;
                    }
                    if let (
                        ScalarType::Struct(structure),
                        ScalarExpression::StructLiteral { fields, .. },
                    ) = (&binding.declared_type, &binding.value)
                    {
                        for field in fields {
                            let declared = self
                                .structs
                                .iter()
                                .find(|item| item.id == *structure)
                                .and_then(|item| {
                                    item.fields.iter().find(|item| item.name == field.name)
                                })
                                .ok_or_else(|| {
                                    self.missing_context(
                                        field.name_span,
                                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                    )
                                })?
                                .clone();
                            if matches!(declared.ty, ScalarType::RuntimeArray { .. }) {
                                let field_id = declared.id.clone();
                                let transfer = match &field.value {
                                    ScalarExpression::Name { name, span } => {
                                        let source = self.binding(name, *span)?;
                                        let source_ty = self.binding_types.get(&source).ok_or_else(|| self.missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding))?;
                                        match copy_policy(source_ty, &self.structs, &self.enums, &self.generic_parameters)
                                            .map_err(|_| self.missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding))? {
                                            CopyPolicy::Copy => RuntimeBackingTransfer::CopyFixed { source },
                                            CopyPolicy::Move => RuntimeBackingTransfer::Move { source },
                                            CopyPolicy::Deferred => return Err(self.missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding)),
                                        }
                                    }
                                    ScalarExpression::Call { .. } => {
                                        let value_point =
                                            self.expression_value_point(&field.value, 0)?;
                                        let call = &self.cfg.points[value_point.0];
                                        RuntimeBackingTransfer::Move {
                                            source: ReferenceBindingId {
                                                source: self.source.clone(),
                                                function_span: self.function_span,
                                                declaration_span: call.source_span.range,
                                                block_span: self.function_span,
                                                kind: ReferenceBindingKind::CallOutput {
                                                    point: call.id,
                                                    output: 0,
                                                },
                                            },
                                        }
                                    }
                                    _ => {
                                        return Err(self.missing_context(
                                            field.span,
                                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                        ))
                                    }
                                };
                                let place = ScalarPlace::Field {
                                    base: Box::new(ScalarPlace::Name {
                                        name: binding.name.clone(),
                                        span: binding.name_span,
                                    }),
                                    field: ScalarFieldReference::Resolved(field_id),
                                    span: field.span,
                                };
                                let target = self.assignment_place(&place, field.span)?;
                                self.point(
                                    CfgPointKind::BackingPlace { target, transfer },
                                    field.span,
                                    scope,
                                    BTreeSet::new(),
                                );
                            }
                            if contains_projected_runtime_array(&declared.ty, &self.structs) {
                                let source =
                                    match &field.value {
                                        ScalarExpression::Name { name, span } => {
                                            self.binding(name, *span)?
                                        }
                                        ScalarExpression::Call { .. } => {
                                            let value_point =
                                                self.expression_value_point(&field.value, 0)?;
                                            let call = &self.cfg.points[value_point.0];
                                            ReferenceBindingId {
                                                source: self.source.clone(),
                                                function_span: self.function_span,
                                                declaration_span: call.source_span.range,
                                                block_span: self.function_span,
                                                kind: ReferenceBindingKind::CallOutput {
                                                    point: call.id,
                                                    output: 0,
                                                },
                                            }
                                        }
                                        _ => return Err(self.missing_context(
                                            field.span,
                                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                        )),
                                    };
                                let place = ScalarPlace::Field {
                                    base: Box::new(ScalarPlace::Name {
                                        name: binding.name.clone(),
                                        span: binding.name_span,
                                    }),
                                    field: ScalarFieldReference::Resolved(declared.id.clone()),
                                    span: field.span,
                                };
                                let target = self.assignment_place(&place, field.span)?;
                                self.point(
                                    CfgPointKind::CopyProjectedBackings { source, target },
                                    field.span,
                                    scope,
                                    BTreeSet::new(),
                                );
                            }
                            if !matches!(
                                declared.ty,
                                ScalarType::Callable { .. } | ScalarType::CheckedReference { .. }
                            ) {
                                continue;
                            }
                            if matches!(&field.value, ScalarExpression::Name { name, .. } if self.module_names.contains(name) && !self.names.contains_key(name) && !self.global_bindings.contains_key(name))
                            {
                                continue;
                            }
                            let rhs_point = if matches!(field.value, ScalarExpression::Call { .. })
                            {
                                self.expression_value_point(&field.value, 0)?
                            } else {
                                self.cfg
                                    .points
                                    .iter()
                                    .rev()
                                    .find(|point| {
                                        point.source_span.range == expression_span(&field.value)
                                            && matches!(
                                                point.kind,
                                                CfgPointKind::Capture { .. }
                                                    | CfgPointKind::Read { .. }
                                                    | CfgPointKind::ReadPlace { .. }
                                                    | CfgPointKind::Borrow { .. }
                                                    | CfgPointKind::Evaluate(_)
                                                    | CfgPointKind::Call { output: 0, .. }
                                            )
                                    })
                                    .map(|point| point.id)
                                    .ok_or_else(|| {
                                        self.missing_context(
                                            field.span,
                                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                        )
                                    })?
                            };
                            let place = ScalarPlace::Field {
                                base: Box::new(ScalarPlace::Name {
                                    name: binding.name.clone(),
                                    span: binding.name_span,
                                }),
                                field: ScalarFieldReference::Resolved(declared.id.clone()),
                                span: field.span,
                            };
                            let event = CfgPointKind::Assign {
                                target: self.assignment_place(&place, field.span)?,
                                rhs_point,
                                previous_origins: BTreeSet::new(),
                                previous_loans: Vec::new(),
                            };
                            let field_assign =
                                if matches!(declared.ty, ScalarType::CheckedReference { .. }) {
                                    self.uncomputed_point(event, field.span, scope);
                                    self.cursor.expect("checked field initializer")
                                } else {
                                    self.point(event, field.span, scope, BTreeSet::new())
                                };
                            self.binding_point(
                                binding,
                                0,
                                ScalarBindingPointRole::FieldAssign(declared.id.clone()),
                                field_assign,
                            )?;
                        }
                    }
                    self.copy_evaluated_aggregate_binding(binding, &id, 0, scope)?;
                    if matches!(
                        binding.declared_type,
                        ScalarType::RuntimeArray { .. } | ScalarType::Array { .. }
                    ) {
                        let transfer = if binding.is_allocation {
                            Some(RuntimeBackingTransfer::Allocate)
                        } else if let ScalarExpression::Name { name, span } = &binding.value {
                            let source = if name == &binding.name {
                                old.as_ref()
                            } else {
                                self.names.get(name)
                            }
                            .ok_or_else(|| {
                                self.missing_context(
                                    *span,
                                    ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                )
                            })?;
                            let ty = self.binding_types.get(source).ok_or_else(|| {
                                self.missing_context(
                                    *span,
                                    ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                )
                            })?;
                            if !self.cfg.backing_parameters.contains(source)
                                && !self.cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Backing { target, .. } if target == source)) {
                                None
                            } else {
                            match copy_policy(
                                ty,
                                &self.structs,
                                &self.enums,
                                &self.generic_parameters,
                            )
                            .map_err(|_| {
                                self.missing_context(
                                    *span,
                                    ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                )
                            })? {
                                CopyPolicy::Copy => Some(RuntimeBackingTransfer::CopyFixed {
                                    source: source.clone(),
                                }),
                                CopyPolicy::Move => Some(RuntimeBackingTransfer::Move {
                                    source: source.clone(),
                                }),
                                CopyPolicy::Deferred => {
                                    return Err(self.missing_context(
                                        *span,
                                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                    ))
                                }
                            }
                            }
                        } else if matches!(binding.declared_type, ScalarType::RuntimeArray { .. })
                            && matches!(binding.value, ScalarExpression::Call { .. })
                        {
                            let call = &self.cfg.points[array_value_point.0];
                            Some(RuntimeBackingTransfer::Move {
                                source: ReferenceBindingId {
                                    source: self.source.clone(),
                                    function_span: self.function_span,
                                    declaration_span: call.source_span.range,
                                    block_span: self.cfg.points[0].source_span.range,
                                    kind: ReferenceBindingKind::CallOutput {
                                        point: call.id,
                                        output: 0,
                                    },
                                },
                            })
                        } else if matches!(binding.declared_type, ScalarType::Array { .. }) {
                            Some(RuntimeBackingTransfer::CreateFixed)
                        } else {
                            None
                        };
                        if let Some(transfer) = transfer {
                            self.point(
                                CfgPointKind::Backing {
                                    target: id.clone(),
                                    transfer,
                                    materialize: matches!(
                                        binding.declared_type,
                                        ScalarType::RuntimeArray { .. }
                                    ),
                                },
                                binding.name_span,
                                scope,
                                BTreeSet::new(),
                            );
                        }
                    }
                    declared.push((binding.name.clone(), id, old));
                    self.additional_receivers(binding, block, scope, &mut declared)?;
                    self.validate_call_receivers(binding)?;
                }
                ScalarBlockItem::Expression(expression) => {
                    let value = self.expression(expression, scope)?;
                    let value_point = self.expression_value_point(expression, 0)?;
                    let outputs = if index >= block.items.len() - block.final_output_values.len() {
                        self.bind_final_output_values(expression, value_point)?
                            .entries
                    } else {
                        Vec::new()
                    };
                    self.move_value(expression, scope)?;
                    if index >= block.items.len() - block.final_output_values.len() {
                        for entry in outputs {
                            if scope == ReferenceScopeId(0)
                                && return_values.len() == receiving_outputs.len()
                            {
                                if let ScalarExpression::Assignment(assignment) = expression {
                                    let result = self
                                        .cfg
                                        .evaluated_values
                                        .results
                                        .iter()
                                        .filter(|result| result.value == entry.value)
                                        .collect::<Vec<_>>();
                                    let [result] = result.as_slice() else {
                                        return Err(self.missing_context(
                                        expression_span(expression), ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                                    ));
                                    };
                                    let snapshot = result.snapshot;
                                    let point = self.point(
                                        CfgPointKind::AssignmentDispose {
                                            snapshot_point: snapshot,
                                        },
                                        expression_span(expression),
                                        scope,
                                        BTreeSet::new(),
                                    );
                                    self.assignment_point(
                                        assignment,
                                        entry.value.resolved.output,
                                        ScalarAssignmentPointRole::Dispose,
                                        point,
                                    );
                                    self.register_final_assignment_consumer(
                                        assignment,
                                        &entry,
                                        &ValueConsumerId {
                                            producer: self.cfg.producer_owner.clone(),
                                            point,
                                            kind: ValueConsumerKind::Discard,
                                        },
                                    )?;
                                }
                                continue;
                            }
                            return_values.push(entry);
                            return_expressions.push(expression);
                        }
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
                    self.assignment(assignment, block.span, scope, &mut declared, false)?;
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
                    self.cfg.while_points.push(ScalarWhileCfgPoint {
                        occurrence: loop_.occurrence.clone(),
                        role: ScalarWhilePointRole::LoopTest,
                        point: loop_head,
                    });
                    self.expression(&loop_.condition, scope)?;
                    let branch = self.point(
                        CfgPointKind::Branch {
                            condition: loop_.condition.clone(),
                        },
                        expression_span(&loop_.condition),
                        scope,
                        BTreeSet::new(),
                    );
                    self.cfg.while_points.push(ScalarWhileCfgPoint {
                        occurrence: loop_.occurrence.clone(),
                        role: ScalarWhilePointRole::Branch,
                        point: branch,
                    });
                    let names = self.names.clone();
                    let origins = self.origins.clone();
                    let checked = self.checked.clone();
                    self.block(&loop_.body)?;
                    let back = self.cursor.expect("loop body terminal");
                    self.cfg.while_points.push(ScalarWhileCfgPoint {
                        occurrence: loop_.occurrence.clone(),
                        role: ScalarWhilePointRole::BodyScopeExit,
                        point: back,
                    });
                    self.cfg.points[back.0].successors.push(loop_head);
                    self.names = names;
                    self.origins = origins;
                    self.checked = checked;
                    self.cursor = Some(branch);
                    self.point(CfgPointKind::Join, loop_.span, scope, BTreeSet::new());
                }
            }
        }
        let evaluated_outputs = return_values.clone();
        if scope == ReferenceScopeId(0) {
            if !block.final_output_values.is_empty()
                && return_values.len() != receiving_outputs.len()
            {
                return Err(self.missing_context(
                    block.span,
                    ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                ));
            }
            for (output, entry) in return_values.into_iter().enumerate() {
                self.validate_final_output_entry(
                    return_expressions[output],
                    &entry,
                    &receiving_outputs[output].ty,
                )?;
                let value_point = entry.value.point;
                let backing_action = if let (
                    ScalarType::RuntimeArray { .. },
                    ScalarExpression::Name { name, span },
                ) =
                    (&receiving_outputs[output].ty, return_expressions[output])
                {
                    if let Some(source) = self.names.get(name) {
                        if let Some(ScalarType::Array { .. }) = self.binding_types.get(source) {
                            let policy = copy_policy(
                                self.binding_types.get(source).expect("typed return source"),
                                &self.structs,
                                &self.enums,
                                &self.generic_parameters,
                            )
                            .map_err(|_| {
                                self.missing_context(
                                    *span,
                                    ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                )
                            })?;
                            match policy {
                                CopyPolicy::Copy => RuntimeReturnBackingAction::CopyFixed {
                                    source: source.clone(),
                                },
                                CopyPolicy::Move => RuntimeReturnBackingAction::MoveFixed {
                                    source: source.clone(),
                                },
                                CopyPolicy::Deferred => {
                                    return Err(self.missing_context(
                                        *span,
                                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                    ))
                                }
                            }
                        } else {
                            RuntimeReturnBackingAction::Transfer
                        }
                    } else {
                        RuntimeReturnBackingAction::Transfer
                    }
                } else {
                    RuntimeReturnBackingAction::Transfer
                };
                self.uncomputed_point(
                    CfgPointKind::ReturnOutput {
                        output,
                        value_point,
                        backing_action,
                        checked: matches!(
                            receiving_outputs[output].ty,
                            ScalarType::CheckedReference { .. }
                        ),
                    },
                    expression_span(return_expressions[output]),
                    scope,
                );
                let consumer = ValueConsumerId {
                    producer: self.cfg.producer_owner.clone(),
                    point: self.cursor.expect("actual root ReturnOutput"),
                    kind: ValueConsumerKind::FinalOutput {
                        block: (*block.occurrence).clone(),
                        output,
                    },
                };
                if let ScalarExpression::Assignment(assignment) = return_expressions[output] {
                    self.register_final_assignment_consumer(assignment, &entry, &consumer)?;
                }
                let source_type = self
                    .value_types
                    .selected_output(
                        &entry.value.resolved.occurrence,
                        entry.value.resolved.output,
                    )
                    .map_err(|source_span| ReferenceAnalysisError::MissingContext {
                        source_span,
                        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    })?;
                let policy = copy_policy(
                    source_type,
                    &self.structs,
                    &self.enums,
                    &self.generic_parameters,
                )
                .map_err(|_| {
                    self.missing_context(
                        entry.value.resolved.occurrence.range,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    )
                })?;
                self.cfg
                    .evaluated_values
                    .disposition_inputs
                    .push(ResolvedValueDispositionInput {
                        source: entry.value.clone(),
                        consumer: consumer.clone(),
                        policy,
                    });
                self.cfg
                    .evaluated_values
                    .final_inputs
                    .push(FinalOutputInput {
                        consumer,
                        source: entry.value,
                        receiving_type: receiving_outputs[output].ty.clone(),
                    });
            }
        }
        let scope_exit = self.point(CfgPointKind::ScopeExit, block.span, scope, BTreeSet::new());
        self.cfg.block_scope_exits.push(ScalarBlockScopeExit {
            occurrence: block.occurrence.as_ref().clone(),
            point: scope_exit,
        });
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
        Ok(ReferenceBlockEvaluation {
            value: result,
            outputs: evaluated_outputs,
            scope_exit,
        })
    }
}

#[derive(Clone, Debug)]
struct HolderProvenanceEntry {
    environment: CallableEnvironmentId,
    holder: CallableHolderPlace,
    producer: HolderProducerRef,
}

#[derive(Clone, Debug, Default)]
pub(super) struct HolderProvenance {
    entries: Vec<HolderProvenanceEntry>,
}

impl PartialEq for HolderProvenance {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl HolderProvenance {
    pub(super) fn lookup(
        &self,
        environment: &CallableEnvironmentId,
        holder: &CallableHolderPlace,
    ) -> Option<&HolderProducerRef> {
        self.entries.iter().find_map(|entry| {
            (entry.environment == *environment && holder_place_same(&entry.holder, holder))
                .then_some(&entry.producer)
        })
    }

    pub(super) fn insert(
        &mut self,
        environment: CallableEnvironmentId,
        holder: CallableHolderPlace,
        producer: HolderProducerRef,
    ) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| {
            entry.environment == environment && holder_place_same(&entry.holder, &holder)
        }) {
            entry.producer = producer;
        } else {
            self.entries.push(HolderProvenanceEntry {
                environment,
                holder,
                producer,
            });
        }
    }

    fn remove(&mut self, environment: &CallableEnvironmentId, holder: &CallableHolderPlace) {
        self.entries.retain(|entry| {
            !(entry.environment == *environment && holder_place_same(&entry.holder, holder))
        });
    }

    fn remove_holder(&mut self, holder: &CallableHolderPlace) {
        self.entries
            .retain(|entry| !holder_place_same(&entry.holder, holder));
    }
}

#[derive(Clone, PartialEq)]
struct ReferenceEdgeState {
    self_frame: FrameVariableRef,
    assignment_applications: Vec<FrozenApplicationState>,
    assignment_destinations: BTreeMap<CfgPointId, Vec<FrozenAssignmentDestination>>,
    assignment_saved: BTreeMap<CfgPointId, Vec<assignment_saved_flow::AssignmentSavedFlowPayload>>,
    allocation_address_values: Vec<Option<ReferenceAllocationAddressValue>>,
    selected_element: Option<(CfgPointId, ReferencePlaceId)>,
    index_values: BTreeMap<ReferenceBindingId, BigInt>,
    transferred_backings: Vec<RuntimeValueId>,
    holder_relations: Vec<CallableHolderRelation>,
    holder_provenance: HolderProvenance,
    holder_value: Vec<CallableHolderRelation>,
    holder_sources: Vec<CallableHolderPlace>,
    holder_point_values: Vec<Vec<CallableHolderRelation>>,
    holder_point_sources: Vec<Vec<CallableHolderPlace>>,
    backing_bindings: BTreeMap<ReferenceBindingId, Vec<RuntimeBackingFact>>,
    backing_projected: Vec<RuntimeProjectedBackingFact>,
    backing_point_values: BTreeMap<CfgPointId, Vec<RuntimeBackingFact>>,
    backing_value: Vec<RuntimeBackingFact>,
    backing_call_paths: Vec<(SourceSpan, Vec<Vec<RuntimeCallArgumentFact>>)>,
    bindings: BTreeMap<ReferenceBindingId, BTreeSet<ReferenceOriginId>>,
    projected: Vec<ReferenceProjectedState>,
    callable_bindings: BTreeMap<ReferenceBindingId, BTreeSet<CallableEnvironmentId>>,
    callable_projected: Vec<CallableProjectedEnvironment>,
    callable_value: BTreeSet<CallableEnvironmentId>,
    callable_value_projected: Vec<CallableProjectedEnvironment>,
    callable_point_values: Vec<BTreeSet<CallableEnvironmentId>>,
    callable_point_projected: Vec<Vec<CallableProjectedEnvironment>>,
    uninitialized: HashSet<ReferenceBindingId>,
    value: Option<BTreeSet<ReferenceOriginId>>,
    released: BTreeSet<ReferenceBindingId>,
    invalidated: BTreeSet<ReferenceBindingId>,
    point_values: Vec<Option<BTreeSet<ReferenceOriginId>>>,
    return_outputs: BTreeMap<usize, BTreeSet<ReferenceOriginId>>,
    moved: Vec<(ReferencePlaceId, SourceSpan)>,
}

#[derive(Clone, PartialEq)]
struct ReferenceWorkItem {
    state: ReferenceEdgeState,
    selected: selected_source::SelectedSourceState,
}

fn admit_reference_work_item(items: &mut Vec<ReferenceWorkItem>, item: ReferenceWorkItem) -> bool {
    if items.contains(&item) {
        return false;
    }
    items.push(item);
    true
}

#[cfg(test)]
#[path = "selected_reach_tests.rs"]
mod selected_reach_tests;
#[cfg(test)]
#[path = "selected_saved_port_tests.rs"]
mod selected_saved_port_tests;

impl ReferenceEdgeState {
    fn new(self_frame: FrameVariableRef) -> Self {
        Self {
            self_frame,
            assignment_applications: Vec::new(),
            assignment_destinations: BTreeMap::new(),
            assignment_saved: BTreeMap::new(),
            allocation_address_values: Vec::new(),
            selected_element: None,
            index_values: BTreeMap::new(),
            transferred_backings: Vec::new(),
            holder_relations: Vec::new(),
            holder_provenance: HolderProvenance::default(),
            holder_value: Vec::new(),
            holder_sources: Vec::new(),
            holder_point_values: Vec::new(),
            holder_point_sources: Vec::new(),
            backing_bindings: BTreeMap::new(),
            backing_projected: Vec::new(),
            backing_point_values: BTreeMap::new(),
            backing_value: Vec::new(),
            backing_call_paths: Vec::new(),
            bindings: BTreeMap::new(),
            projected: Vec::new(),
            callable_bindings: BTreeMap::new(),
            callable_projected: Vec::new(),
            callable_value: BTreeSet::new(),
            callable_value_projected: Vec::new(),
            callable_point_values: Vec::new(),
            callable_point_projected: Vec::new(),
            uninitialized: HashSet::new(),
            value: None,
            released: BTreeSet::new(),
            invalidated: BTreeSet::new(),
            point_values: Vec::new(),
            return_outputs: BTreeMap::new(),
            moved: Vec::new(),
        }
    }
}

fn global_callable_places(
    cfg: &ScalarReferenceCfg,
    state: &ReferenceEdgeState,
) -> Vec<CallableHolderPlace> {
    let mut places = Vec::new();
    let module_bindings = cfg
        .top_level
        .bindings
        .iter()
        .flat_map(|entry| match &entry.execution {
            ScalarTopLevelBindingExecution::Connected { points } => points
                .iter()
                .filter_map(|point| match &cfg.points[point.0].kind {
                    CfgPointKind::Bind { binding } => Some(binding.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();
    for binding in &module_bindings {
        places.push(CallableHolderPlace::Binding(binding.clone()));
    }
    let function_span = cfg.points[0].source_span.range;
    for point in &cfg.initial_points {
        let binding = match &point.kind {
            CfgPointKind::Read { binding } => Some(binding),
            CfgPointKind::ReadPlace { candidates, .. } => {
                for candidate in candidates {
                    if candidate.binding.function_span != function_span
                        && candidate.binding.kind == ReferenceBindingKind::Declared
                    {
                        let holder = if candidate.projections.is_empty() {
                            CallableHolderPlace::Binding(candidate.binding.clone())
                        } else {
                            CallableHolderPlace::Projection(candidate.clone())
                        };
                        if !places.contains(&holder) {
                            places.push(holder);
                        }
                    }
                }
                None
            }
            CfgPointKind::Assign { target, .. } => Some(&target.binding),
            _ => None,
        };
        if let Some(binding) = binding {
            if binding.function_span != function_span
                && binding.kind == ReferenceBindingKind::Declared
            {
                let holder = CallableHolderPlace::Binding(binding.clone());
                if !places.contains(&holder) {
                    places.push(holder);
                }
            }
        }
    }
    for relation in &state.holder_relations {
        for holder in &relation.holders {
            let binding = match holder {
                CallableHolderPlace::Binding(binding) => Some(binding),
                CallableHolderPlace::Projection(place) => Some(&place.binding),
                _ => None,
            };
            if binding.is_some_and(|binding| {
                (binding.function_span != function_span || module_bindings.contains(binding))
                    && binding.kind == ReferenceBindingKind::Declared
            }) && !places.contains(holder)
            {
                places.push(holder.clone());
            }
        }
    }
    places
}

fn global_callable_call_paths(
    cfg: &ScalarReferenceCfg,
    state: &ReferenceEdgeState,
    callee: &ResolvedCallTarget,
) -> Vec<GlobalCallableCallPathFact> {
    let mut places = global_callable_places(cfg, state);
    if let ResolvedCallTarget::CallableField {
        receiver_place,
        field,
    } = callee
    {
        if receiver_place.binding.function_span != cfg.points[0].source_span.range {
            let mut place = receiver_place.clone();
            place.projections.push(ReferencePlaceProjection::Field(
                ScalarFieldReference::Resolved(field.clone()),
            ));
            let holder = CallableHolderPlace::Projection(place);
            if !places.contains(&holder) {
                places.push(holder);
            }
        }
    }
    places
        .into_iter()
        .map(|holder| {
            let binding = match &holder {
                CallableHolderPlace::Binding(binding) => binding,
                CallableHolderPlace::Projection(place) => &place.binding,
                _ => unreachable!("global holder place"),
            };
            let relations = state
                .holder_relations
                .iter()
                .filter(|relation| relation.holders.contains(&holder))
                .cloned()
                .collect::<Vec<_>>();
            GlobalCallableCallPathFact {
                callee: callee.clone(),
                holder: holder.clone(),
                initialization: if !relations.is_empty() && !state.uninitialized.contains(binding) {
                    GlobalCallableInitializationFact::Initialized
                } else if cfg.top_level.bindings.iter().any(|entry| matches!(&entry.execution, ScalarTopLevelBindingExecution::Connected { points } if points.iter().any(|point| matches!(&cfg.points[point.0].kind, CfgPointKind::Bind { binding: known } if known == binding)))) {
                    GlobalCallableInitializationFact::Uninitialized
                } else {
                    GlobalCallableInitializationFact::AwaitingCallsiteInput
                },
                relations,
                backings: state
                    .backing_bindings
                    .get(binding)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect(),
                projected_backings: state
                    .backing_projected
                    .iter()
                    .filter(|fact| fact.place.binding == *binding)
                    .cloned()
                    .collect(),
                origins: state
                    .bindings
                    .get(binding)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect(),
                released: state.released.contains(binding),
                moved: state
                    .moved
                    .iter()
                    .any(|(place, _)| place.binding == *binding),
            }
        })
        .collect()
}

fn seed_global_callsite_entry(
    cfg: &ScalarReferenceCfg,
    state: &mut ReferenceEdgeState,
    path: &GlobalCallableCallInputFact,
) -> Result<(), ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: path.call.clone(),
        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
    };
    for holder_fact in &path.holders {
        let binding = match &holder_fact.holder {
            CallableHolderPlace::Binding(binding) => binding.clone(),
            CallableHolderPlace::Projection(place) => place.binding.clone(),
            _ => return Err(invalid()),
        };
        match holder_fact.initialization {
            GlobalCallableInitializationFact::Uninitialized => {
                let initialized = path.holders.iter().any(|known| {
                    matches!(
                        known.initialization,
                        GlobalCallableInitializationFact::Initialized
                    ) && match &known.holder {
                        CallableHolderPlace::Binding(known) => known == &binding,
                        CallableHolderPlace::Projection(place) => place.binding == binding,
                        _ => false,
                    }
                });
                if !initialized {
                    state.uninitialized.insert(binding);
                }
                continue;
            }
            GlobalCallableInitializationFact::AwaitingCallsiteInput => continue,
            GlobalCallableInitializationFact::Initialized => {}
        }
        for origin in &holder_fact.origins {
            if cfg.facts.get(origin.0).is_none() {
                return Err(invalid());
            }
        }
        for relation in &holder_fact.relations {
            let fact = cfg
                .environments
                .get(relation.environment.index)
                .ok_or_else(invalid)?;
            if fact.literal != relation.environment.literal {
                return Err(invalid());
            }
            for alternatives in &relation.capture_paths {
                for input in alternatives {
                    for id in input
                        .environments
                        .iter()
                        .chain(input.projected.iter().flat_map(|known| &known.environments))
                    {
                        let fact = cfg.environments.get(id.index).ok_or_else(invalid)?;
                        if fact.literal != id.literal {
                            return Err(invalid());
                        }
                    }
                    for origin in &input.origins {
                        if cfg.facts.get(origin.0).is_none() {
                            return Err(invalid());
                        }
                    }
                }
            }
        }
        if !holder_fact.origins.is_empty() {
            state
                .bindings
                .entry(binding.clone())
                .or_default()
                .extend(holder_fact.origins.iter().cloned());
        }
        for relation in &holder_fact.relations {
            match &holder_fact.holder {
                CallableHolderPlace::Binding(_) => {
                    state
                        .callable_bindings
                        .entry(binding.clone())
                        .or_default()
                        .insert(relation.environment.clone());
                }
                CallableHolderPlace::Projection(place) => {
                    let projected = CallableProjectedEnvironment {
                        place: place.clone(),
                        environments: BTreeSet::from([relation.environment.clone()]),
                    };
                    if let Some(known) = state
                        .callable_projected
                        .iter_mut()
                        .find(|known| reference_places_same(&known.place, &projected.place))
                    {
                        known.environments.insert(relation.environment.clone());
                    } else {
                        state.callable_projected.push(projected);
                    }
                }
                _ => return Err(invalid()),
            }
            if !state.holder_relations.contains(relation) {
                state.holder_relations.push(relation.clone());
            }
            state.holder_provenance.insert(
                relation.environment.clone(),
                holder_fact.holder.clone(),
                HolderProducerRef::GlobalCallsiteInput {
                    call: path.call.clone(),
                    holder: holder_fact.holder.clone(),
                    environment: relation.environment.clone(),
                },
            );
        }
        for backing in &holder_fact.backings {
            let entry = state
                .backing_bindings
                .entry(backing.owner.clone())
                .or_default();
            if !entry.contains(backing) {
                entry.push(backing.clone());
            }
        }
        for projected in &holder_fact.projected_backings {
            if !state.backing_projected.contains(projected) {
                state.backing_projected.push(projected.clone());
            }
        }
        if holder_fact.released {
            state.released.insert(binding.clone());
        }
        if holder_fact.moved {
            let place = match &holder_fact.holder {
                CallableHolderPlace::Binding(known) => ReferencePlaceId {
                    source: known.source.clone(),
                    function_span: known.function_span,
                    declaration_span: known.declaration_span,
                    block_span: known.block_span,
                    binding: known.clone(),
                    projections: Vec::new(),
                    place: ScalarPlace::Name {
                        name: String::new(),
                        span: known.declaration_span,
                    },
                },
                CallableHolderPlace::Projection(known) => known.clone(),
                _ => return Err(invalid()),
            };
            if !state.moved.iter().any(|(known, _)| known == &place) {
                state.moved.push((place, path.call.clone()));
            }
        }
    }
    normalize_holder_relations(&mut state.holder_relations);
    Ok(())
}

fn push_callable_array_cleanup(
    actions: &mut Vec<CallableHolderAction>,
    errors: &mut Vec<ReferenceAnalysisError>,
    backing: &RuntimeBackingFact,
    span: &SourceSpan,
) {
    if backing.kind != RuntimeBackingKind::OwnedAllocation || backing.callable_family.is_empty() {
        return;
    }
    if !validate_callable_array_cleanup(errors, backing, span) {
        return;
    }
    let action = CallableHolderAction::DropArrayElements {
        backing: backing.clone(),
        span: span.clone(),
    };
    if !actions.contains(&action) {
        actions.push(action);
    }
}

fn validate_callable_array_cleanup(
    errors: &mut Vec<ReferenceAnalysisError>,
    backing: &RuntimeBackingFact,
    span: &SourceSpan,
) -> bool {
    if backing.callable_family.iter().any(|family| {
        family.partitions.is_empty()
            || family.partitions.iter().any(|partition| {
                partition.remainder.alternatives.is_empty()
                    || partition.representatives.iter().any(|representative| representative.cell.alternatives.is_empty())
                    || partition.remainder.alternatives.iter()
                        .chain(partition.representatives.iter().flat_map(|representative| &representative.cell.alternatives))
                        .any(|case| matches!(case, CallableCellCaseFact::Initialized { backing: value, owner, .. }
                            if value != &backing.value || owner != &backing.owner))
            })
    }) {
        errors.push(ReferenceAnalysisError::MissingContext {
            source_span: span.clone(),
            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
        });
        return false;
    }
    true
}

fn holder_place_cmp(left: &CallableHolderPlace, right: &CallableHolderPlace) -> std::cmp::Ordering {
    fn projection_key(projections: &[ReferencePlaceProjection]) -> Vec<(u8, u32, u32)> {
        projections
            .iter()
            .map(|projection| match projection {
                ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)) => {
                    (0, field.structure.index as u32, field.index as u32)
                }
                ReferencePlaceProjection::Field(ScalarFieldReference::Unresolved {
                    span, ..
                }) => (1, span.start, span.end),
                ReferencePlaceProjection::Index(expression) => {
                    let span = expression_span(expression);
                    (2, span.start, span.end)
                }
                ReferencePlaceProjection::Dereference(loan) => {
                    (3, loan.creation_span.start, loan.creation_span.end)
                }
            })
            .collect()
    }
    match (left, right) {
        (CallableHolderPlace::Temporary(a), CallableHolderPlace::Temporary(b)) => a.cmp(b),
        (CallableHolderPlace::Binding(a), CallableHolderPlace::Binding(b)) => a.cmp(b),
        (CallableHolderPlace::Projection(a), CallableHolderPlace::Projection(b)) => a
            .binding
            .cmp(&b.binding)
            .then_with(|| projection_key(&a.projections).cmp(&projection_key(&b.projections))),
        (CallableHolderPlace::Output(a), CallableHolderPlace::Output(b)) => a.cmp(b),
        (
            CallableHolderPlace::OutputProjection {
                output: a,
                projections: left,
            },
            CallableHolderPlace::OutputProjection {
                output: b,
                projections: right,
            },
        ) => a
            .cmp(b)
            .then_with(|| projection_key(left).cmp(&projection_key(right))),
        (CallableHolderPlace::Temporary(_), _) => std::cmp::Ordering::Less,
        (_, CallableHolderPlace::Temporary(_)) => std::cmp::Ordering::Greater,
        (CallableHolderPlace::Binding(_), _) => std::cmp::Ordering::Less,
        (_, CallableHolderPlace::Binding(_)) => std::cmp::Ordering::Greater,
        (CallableHolderPlace::Projection(_), _) => std::cmp::Ordering::Less,
        (_, CallableHolderPlace::Projection(_)) => std::cmp::Ordering::Greater,
        (CallableHolderPlace::Output(_), _) => std::cmp::Ordering::Less,
        (_, CallableHolderPlace::Output(_)) => std::cmp::Ordering::Greater,
    }
}

pub(super) fn holder_place_same(left: &CallableHolderPlace, right: &CallableHolderPlace) -> bool {
    match (left, right) {
        (CallableHolderPlace::Projection(left), CallableHolderPlace::Projection(right)) => {
            reference_places_same(left, right)
        }
        _ => left == right,
    }
}

fn normalize_holder_relations(relations: &mut Vec<CallableHolderRelation>) {
    for relation in relations.iter_mut() {
        relation.holders.sort_by(holder_place_cmp);
        relation
            .holders
            .dedup_by(|left, right| holder_place_same(left, right));
    }
    relations.retain(|relation| !relation.holders.is_empty());
    relations.sort_by(|left, right| {
        holder_place_cmp(&left.holders[0], &right.holders[0])
            .then_with(|| left.environment.cmp(&right.environment))
    });
}

fn replicate_global_struct_holders(
    source_points: &[&ReferenceFlowPoint],
    state: &mut ReferenceEdgeState,
    target: &ReferenceBindingId,
    span: &SourceSpan,
    actions: &mut Vec<CallableHolderAction>,
    point: CfgPointId,
    errors: &mut Vec<ReferenceAnalysisError>,
) {
    let source = source_points
        .iter()
        .find_map(|candidate| match &candidate.kind {
            CfgPointKind::Read { binding } => {
                Some((binding.clone(), candidate.source_span.clone()))
            }
            CfgPointKind::ReadPlace { candidates, .. } if candidates.len() == 1 => {
                Some((candidates[0].binding.clone(), candidate.source_span.clone()))
            }
            _ => None,
        });
    let Some((source_binding, read_span)) = source else {
        return;
    };
    if source_binding.function_span != ByteSpan::new(0, 0)
        || source_binding.kind != ReferenceBindingKind::Declared
    {
        return;
    }
    if state.uninitialized.contains(&source_binding) {
        errors.push(ReferenceAnalysisError::MissingOrigin {
            source_span: read_span,
            binding_id: source_binding,
            reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
        });
        return;
    }
    let mut sources: Vec<CallableHolderPlace> = state
        .holder_relations
        .iter()
        .flat_map(|relation| relation.holders.iter())
        .filter(|holder| match holder {
            CallableHolderPlace::Projection(place) => place.binding == source_binding,
            _ => false,
        })
        .cloned()
        .collect();
    sources.sort_by(holder_place_cmp);
    sources.dedup_by(|left, right| holder_place_same(left, right));
    for source in sources {
        let CallableHolderPlace::Projection(place) = &source else {
            continue;
        };
        let mut target_place = place.clone();
        target_place.source = target.source.clone();
        target_place.function_span = target.function_span;
        target_place.declaration_span = target.declaration_span;
        target_place.block_span = target.block_span;
        target_place.binding = target.clone();
        transfer_holder(
            &mut state.holder_relations,
            &source,
            CallableHolderPlace::Projection(target_place),
            false,
            span,
            actions,
            point,
            &mut state.holder_provenance,
        );
    }
}

fn callable_array_elements(
    environments: &[CallableEnvironmentFact],
    state: &ReferenceEdgeState,
    binding: &ReferenceBindingId,
) -> Vec<CallableArrayElementFact> {
    let mut elements = Vec::new();
    for relation in &state.holder_relations {
        for holder in &relation.holders {
            let CallableHolderPlace::Projection(place) = holder else {
                continue;
            };
            if place.binding != *binding {
                continue;
            }
            let Some(index) =
                place
                    .projections
                    .iter()
                    .rev()
                    .find_map(|projection| match projection {
                        ReferencePlaceProjection::Index(ScalarExpression::Integer {
                            value,
                            ..
                        }) => Some(CallableArrayIndexFact::Constant(value.clone())),
                        _ => None,
                    })
            else {
                continue;
            };
            let mut aliases = relation.holders.clone();
            aliases.sort_by(holder_place_cmp);
            aliases.dedup_by(|left, right| holder_place_same(left, right));
            let element = CallableArrayElementFact {
                place: place.clone(),
                index,
                environment: relation.environment.clone(),
                instances: CallableArrayEnvironmentInstances::KnownElement,
                aliases,
                loans: environments[relation.environment.index].loans.clone(),
            };
            if !elements.contains(&element) {
                elements.push(element);
            }
        }
    }
    elements.sort_by(|left, right| {
        holder_place_cmp(
            &CallableHolderPlace::Projection(left.place.clone()),
            &CallableHolderPlace::Projection(right.place.clone()),
        )
        .then_with(|| left.environment.cmp(&right.environment))
    });
    elements
}

fn normalize_array_family(family: &mut CallableArrayFamilyFact) {
    family.elements.sort_by(|left, right| {
        holder_place_cmp(
            &CallableHolderPlace::Projection(left.place.clone()),
            &CallableHolderPlace::Projection(right.place.clone()),
        )
        .then_with(|| left.environment.cmp(&right.environment))
    });
    family.elements.dedup();
    family.unselected.selected.clear();
    for element in &family.elements {
        if !family.unselected.selected.contains(&element.index) {
            family.unselected.selected.push(element.index.clone());
        }
    }
    family.index_relations.clear();
    for (position, left) in family.elements.iter().enumerate() {
        for right in &family.elements[position + 1..] {
            let relation = match (&left.index, &right.index) {
                (
                    CallableArrayIndexFact::Constant(left),
                    CallableArrayIndexFact::Constant(right),
                ) if left == right => CallableArrayIndexRelation::Equal,
                (CallableArrayIndexFact::Constant(_), CallableArrayIndexFact::Constant(_)) => {
                    CallableArrayIndexRelation::Distinct
                }
                _ => CallableArrayIndexRelation::MayAlias,
            };
            let relation = CallableArrayIndexRelationFact {
                left: left.index.clone(),
                right: right.index.clone(),
                relation,
            };
            if !family.index_relations.contains(&relation) {
                family.index_relations.push(relation);
            }
        }
    }
}

fn seed_indexed_family(backing: &mut RuntimeBackingFact, environments: &[CallableEnvironmentFact]) {
    let backing_value = backing.value.clone();
    let owner = backing.owner.clone();
    for family in &mut backing.callable_family {
        let partition = &mut family.partitions[0];
        for element in &family.elements {
            let CallableArrayIndexFact::Constant(value) = &element.index else {
                continue;
            };
            let case = CallableCellCaseFact::Initialized {
                environment_origin: element.environment.clone(),
                instance: CallableEnvironmentInstanceClassFact::ImportedMayAlias,
                holder_aliases: element.aliases.clone(),
                backing: backing_value.clone(),
                owner: owner.clone(),
                capture_paths: environments[element.environment.index]
                    .capture_paths
                    .clone(),
                loans: element.loans.clone(),
                moved: false,
                origin: element.environment.literal.clone(),
            };
            let key = CallableIndexKeyFact::Constant(value.clone());
            if let Some(representative) = partition
                .representatives
                .iter_mut()
                .find(|known| known.key == key)
            {
                if !representative.cell.alternatives.contains(&case) {
                    representative.cell.alternatives.push(case);
                }
            } else {
                partition
                    .representatives
                    .push(CallableIndexRepresentativeFact {
                        key,
                        anchors: Vec::new(),
                        cell: CallableCellDomainFact {
                            alternatives: vec![case],
                        },
                    });
            }
        }
        partition
            .representatives
            .sort_by(|left, right| match (&left.key, &right.key) {
                (CallableIndexKeyFact::Constant(left), CallableIndexKeyFact::Constant(right)) => {
                    left.cmp(right)
                }
                _ => std::cmp::Ordering::Equal,
            });
        partition.relations = partition
            .representatives
            .iter()
            .enumerate()
            .flat_map(|(position, left)| {
                partition.representatives[position + 1..]
                    .iter()
                    .map(move |right| CallableIndexEqualityFact {
                        left: left.key.clone(),
                        right: right.key.clone(),
                        relation: CallableArrayIndexRelation::Distinct,
                    })
            })
            .collect();
    }
}

fn record_known_indexed_element(
    family: &mut CallableArrayFamilyFact,
    element: &CallableArrayElementFact,
    backing: &RuntimeValueId,
    owner: &ReferenceBindingId,
    capture_paths: Vec<Vec<CallableCaptureInput>>,
) {
    let CallableArrayIndexFact::Constant(index) = &element.index else {
        return;
    };
    let case = CallableCellCaseFact::Initialized {
        environment_origin: element.environment.clone(),
        instance: CallableEnvironmentInstanceClassFact::ImportedMayAlias,
        holder_aliases: element.aliases.clone(),
        backing: backing.clone(),
        owner: owner.clone(),
        capture_paths,
        loans: element.loans.clone(),
        moved: false,
        origin: element.environment.literal.clone(),
    };
    let key = CallableIndexKeyFact::Constant(index.clone());
    for partition in &mut family.partitions {
        if let Some(representative) = partition
            .representatives
            .iter_mut()
            .find(|known| known.key == key)
        {
            representative.cell.alternatives = vec![case.clone()];
        } else {
            partition
                .representatives
                .push(CallableIndexRepresentativeFact {
                    key: key.clone(),
                    anchors: Vec::new(),
                    cell: CallableCellDomainFact {
                        alternatives: vec![case.clone()],
                    },
                });
            partition
                .representatives
                .sort_by(|left, right| match (&left.key, &right.key) {
                    (
                        CallableIndexKeyFact::Constant(left),
                        CallableIndexKeyFact::Constant(right),
                    ) => left.cmp(right),
                    _ => std::cmp::Ordering::Equal,
                });
        }
    }
    normalize_indexed_partitions(family);
}

fn same_nonfamily_state(left: &ReferenceEdgeState, right: &ReferenceEdgeState) -> bool {
    if !right.backing_bindings.values().flatten().any(|backing| {
        backing.kind == RuntimeBackingKind::OwnedAllocation && !backing.callable_family.is_empty()
    }) {
        return left == right;
    }
    let mut left = left.clone();
    let mut right = right.clone();
    for state in [&mut left, &mut right] {
        for backings in state.backing_bindings.values_mut() {
            for backing in backings {
                backing.callable_family.clear();
            }
        }
        for backing in &mut state.backing_value {
            backing.callable_family.clear();
        }
        for backings in state.backing_point_values.values_mut() {
            for backing in backings {
                backing.callable_family.clear();
            }
        }
        for projected in &mut state.backing_projected {
            projected.backing.callable_family.clear();
        }
        for (_, paths) in &mut state.backing_call_paths {
            for argument in paths.iter_mut().flatten() {
                argument.backing.callable_family.clear();
            }
        }
    }
    left == right
}

fn merge_indexed_partition(
    target: &mut CallableIndexedPartitionFact,
    incoming: &CallableIndexedPartitionFact,
) -> bool {
    let mut changed = false;
    for (target, incoming) in target
        .representatives
        .iter_mut()
        .zip(&incoming.representatives)
    {
        for case in &incoming.cell.alternatives {
            if !target.cell.alternatives.contains(case) {
                target.cell.alternatives.push(case.clone());
                changed = true;
            }
        }
    }
    for case in &incoming.remainder.alternatives {
        if !target.remainder.alternatives.contains(case) {
            target.remainder.alternatives.push(case.clone());
            changed = true;
        }
    }
    changed
}

fn join_backing_families(target: &mut RuntimeBackingFact, incoming: &RuntimeBackingFact) -> bool {
    let mut changed = false;
    for (target, incoming) in target
        .callable_family
        .iter_mut()
        .zip(&incoming.callable_family)
    {
        for element in &incoming.elements {
            if !target.elements.contains(element) {
                target.elements.push(element.clone());
                changed = true;
            }
        }
        for partition in &incoming.partitions {
            if let Some(existing) = target.partitions.iter_mut().find(|existing| {
                existing.relations == partition.relations
                    && existing
                        .representatives
                        .iter()
                        .map(|rep| (&rep.key, &rep.anchors))
                        .eq(partition
                            .representatives
                            .iter()
                            .map(|rep| (&rep.key, &rep.anchors)))
            }) {
                changed |= merge_indexed_partition(existing, partition);
            } else {
                target.partitions.push(partition.clone());
                changed = true;
            }
        }
        if changed {
            normalize_array_family(target);
            normalize_indexed_partitions(target);
        }
    }
    changed
}

fn join_state_families(target: &mut ReferenceEdgeState, incoming: &ReferenceEdgeState) -> bool {
    let mut changed = false;
    for (binding, facts) in &incoming.backing_bindings {
        for fact in facts {
            let target_fact = target
                .backing_bindings
                .get_mut(binding)
                .expect("matching backing binding")
                .iter_mut()
                .find(|known| {
                    known.value == fact.value
                        && known.owner == fact.owner
                        && known.length == fact.length
                })
                .expect("matching correlated backing");
            changed |= join_backing_families(target_fact, fact);
        }
    }
    for (target, incoming) in target.backing_value.iter_mut().zip(&incoming.backing_value) {
        changed |= join_backing_families(target, incoming);
    }
    for (point, facts) in &incoming.backing_point_values {
        for (target, incoming) in target
            .backing_point_values
            .get_mut(point)
            .expect("matching backing point")
            .iter_mut()
            .zip(facts)
        {
            changed |= join_backing_families(target, incoming);
        }
    }
    for (target, incoming) in target
        .backing_projected
        .iter_mut()
        .zip(&incoming.backing_projected)
    {
        changed |= join_backing_families(&mut target.backing, &incoming.backing);
    }
    for ((_, target_paths), (_, incoming_paths)) in target
        .backing_call_paths
        .iter_mut()
        .zip(&incoming.backing_call_paths)
    {
        for (target, incoming) in target_paths
            .iter_mut()
            .flatten()
            .zip(incoming_paths.iter().flatten())
        {
            changed |= join_backing_families(&mut target.backing, &incoming.backing);
        }
    }
    changed
}

fn index_key_cmp(left: &CallableIndexKeyFact, right: &CallableIndexKeyFact) -> std::cmp::Ordering {
    match (left, right) {
        (CallableIndexKeyFact::Constant(left), CallableIndexKeyFact::Constant(right)) => {
            left.cmp(right)
        }
        (CallableIndexKeyFact::Constant(_), _) => std::cmp::Ordering::Less,
        (_, CallableIndexKeyFact::Constant(_)) => std::cmp::Ordering::Greater,
        (CallableIndexKeyFact::LivePlace(left), CallableIndexKeyFact::LivePlace(right)) => {
            left.binding.cmp(&right.binding).then_with(|| {
                holder_place_cmp(
                    &CallableHolderPlace::Projection(left.clone()),
                    &CallableHolderPlace::Projection(right.clone()),
                )
            })
        }
        (CallableIndexKeyFact::LivePlace(_), _) => std::cmp::Ordering::Less,
        (_, CallableIndexKeyFact::LivePlace(_)) => std::cmp::Ordering::Greater,
        (
            CallableIndexKeyFact::Evaluated {
                point: left,
                occurrence: left_span,
                function_span: left_function,
            },
            CallableIndexKeyFact::Evaluated {
                point: right,
                occurrence: right_span,
                function_span: right_function,
            },
        ) => left_function
            .start
            .cmp(&right_function.start)
            .then_with(|| left.cmp(right))
            .then_with(|| left_span.range.start.cmp(&right_span.range.start)),
    }
}

fn normalize_indexed_partition(partition: &mut CallableIndexedPartitionFact) -> bool {
    let count = partition.representatives.len();
    let keys = partition
        .representatives
        .iter()
        .map(|rep| rep.key.clone())
        .collect::<Vec<_>>();
    let mut classes = (0..count).collect::<Vec<_>>();
    for relation in &partition.relations {
        let left = keys
            .iter()
            .position(|key| *key == relation.left)
            .expect("known indexed relation left");
        let right = keys
            .iter()
            .position(|key| *key == relation.right)
            .expect("known indexed relation right");
        if relation.relation == CallableArrayIndexRelation::Equal {
            let from = classes[right];
            let to = classes[left];
            for class in &mut classes {
                if *class == from {
                    *class = to;
                }
            }
        }
    }
    for (position, left) in partition.representatives.iter().enumerate() {
        if let CallableIndexKeyFact::Constant(left_value) = &left.key {
            for (other, right) in partition
                .representatives
                .iter()
                .enumerate()
                .skip(position + 1)
            {
                if let CallableIndexKeyFact::Constant(right_value) = &right.key {
                    if classes[position] == classes[other] && left_value != right_value {
                        return false;
                    }
                }
            }
        }
    }
    let mut distinct = Vec::new();
    for relation in &partition.relations {
        if relation.relation != CallableArrayIndexRelation::Distinct {
            continue;
        }
        let left = keys
            .iter()
            .position(|key| *key == relation.left)
            .expect("known indexed relation left");
        let right = keys
            .iter()
            .position(|key| *key == relation.right)
            .expect("known indexed relation right");
        if classes[left] == classes[right] {
            return false;
        }
        distinct.push((classes[left], classes[right]));
    }
    for left in 0..count {
        let mut cases = Vec::new();
        for (right, representative) in partition.representatives.iter().enumerate() {
            if classes[left] == classes[right] {
                for case in &representative.cell.alternatives {
                    if !cases.contains(case) {
                        cases.push(case.clone());
                    }
                }
            }
        }
        partition.representatives[left].cell.alternatives = cases;
    }
    partition
        .representatives
        .sort_by(|left, right| index_key_cmp(&left.key, &right.key));
    let mut sorted_classes = Vec::new();
    for representative in &partition.representatives {
        let original = keys
            .iter()
            .position(|key| *key == representative.key)
            .expect("known indexed representative");
        sorted_classes.push(classes[original]);
    }
    partition.relations.clear();
    for (left, representative) in partition.representatives.iter().enumerate() {
        for (right, other) in partition.representatives.iter().enumerate().skip(left + 1) {
            let relation = if sorted_classes[left] == sorted_classes[right] {
                Some(CallableArrayIndexRelation::Equal)
            } else if distinct.iter().any(|(a, b)| (*a == sorted_classes[left] && *b == sorted_classes[right])
                || (*b == sorted_classes[left] && *a == sorted_classes[right]))
                || partition.representatives.iter().enumerate().any(|(first, left_rep)| {
                    sorted_classes[first] == sorted_classes[left]
                        && partition.representatives.iter().enumerate().any(|(second, right_rep)| {
                            sorted_classes[second] == sorted_classes[right]
                                && matches!((&left_rep.key, &right_rep.key),
                                    (CallableIndexKeyFact::Constant(a), CallableIndexKeyFact::Constant(b)) if a != b)
                        })
                })
            {
                Some(CallableArrayIndexRelation::Distinct)
            } else {
                None
            };
            if let Some(relation) = relation {
                partition.relations.push(CallableIndexEqualityFact {
                    left: representative.key.clone(),
                    right: other.key.clone(),
                    relation,
                });
            }
        }
    }
    true
}

fn normalize_indexed_partitions(family: &mut CallableArrayFamilyFact) {
    let mut normalized = Vec::new();
    for mut partition in std::mem::take(&mut family.partitions) {
        if normalize_indexed_partition(&mut partition) && !normalized.contains(&partition) {
            normalized.push(partition);
        }
    }
    family.partitions = normalized;
}

fn select_indexed_partition(
    partition: &CallableIndexedPartitionFact,
    key: CallableIndexKeyFact,
    anchor: CallableIndexAnchorFact,
) -> Vec<CallableIndexedPartitionFact> {
    let mut normalized = partition.clone();
    if !normalize_indexed_partition(&mut normalized) {
        return Vec::new();
    }
    let partition = &normalized;
    if partition
        .representatives
        .iter()
        .any(|representative| representative.key == key)
    {
        return vec![normalized];
    }
    let mut alternatives = Vec::new();
    for representative in &partition.representatives {
        let mut next = partition.clone();
        next.representatives.push(CallableIndexRepresentativeFact {
            key: key.clone(),
            anchors: vec![anchor.clone()],
            cell: representative.cell.clone(),
        });
        next.relations.push(CallableIndexEqualityFact {
            left: key.clone(),
            right: representative.key.clone(),
            relation: CallableArrayIndexRelation::Equal,
        });
        let mut branches = vec![next];
        for known in &partition.representatives {
            let mut refined = Vec::new();
            for branch in branches {
                for relation in [
                    CallableArrayIndexRelation::Equal,
                    CallableArrayIndexRelation::Distinct,
                ] {
                    let mut candidate = branch.clone();
                    candidate.relations.push(CallableIndexEqualityFact {
                        left: key.clone(),
                        right: known.key.clone(),
                        relation,
                    });
                    if normalize_indexed_partition(&mut candidate) && !refined.contains(&candidate)
                    {
                        refined.push(candidate);
                    }
                }
            }
            branches = refined;
        }
        for branch in branches {
            if !alternatives.contains(&branch) {
                alternatives.push(branch);
            }
        }
    }
    let mut next = partition.clone();
    next.representatives.push(CallableIndexRepresentativeFact {
        key: key.clone(),
        anchors: vec![anchor],
        cell: partition.remainder.clone(),
    });
    for known in &partition.representatives {
        next.relations.push(CallableIndexEqualityFact {
            left: key.clone(),
            right: known.key.clone(),
            relation: CallableArrayIndexRelation::Distinct,
        });
    }
    if normalize_indexed_partition(&mut next) && !alternatives.contains(&next) {
        alternatives.push(next);
    }
    alternatives
}

fn evaluate_indexed_family(
    family: &mut CallableArrayFamilyFact,
    evaluated: CallableIndexKeyFact,
    live: Option<CallableIndexKeyFact>,
    constant: Option<BigInt>,
    point: CfgPointId,
) {
    let mut alternatives = Vec::new();
    for partition in &family.partitions {
        let live_alternatives = if let Some(live) = &live {
            let anchor = match live {
                CallableIndexKeyFact::LivePlace(place) => {
                    CallableIndexAnchorFact::ValuePlace(place.clone())
                }
                _ => unreachable!(),
            };
            if let Some(value) = &constant {
                select_indexed_partition(
                    partition,
                    CallableIndexKeyFact::Constant(value.clone()),
                    anchor.clone(),
                )
                .into_iter()
                .map(|mut partition| {
                    let known = CallableIndexKeyFact::Constant(value.clone());
                    if !partition.representatives.iter().any(|rep| rep.key == *live) {
                        let cell = partition
                            .representatives
                            .iter()
                            .find(|rep| rep.key == known)
                            .expect("constant representative")
                            .cell
                            .clone();
                        partition
                            .representatives
                            .push(CallableIndexRepresentativeFact {
                                key: live.clone(),
                                anchors: vec![anchor.clone()],
                                cell,
                            });
                        partition.relations.push(CallableIndexEqualityFact {
                            left: live.clone(),
                            right: known,
                            relation: CallableArrayIndexRelation::Equal,
                        });
                    }
                    partition
                })
                .collect()
            } else {
                select_indexed_partition(partition, live.clone(), anchor)
            }
        } else {
            if let Some(value) = &constant {
                select_indexed_partition(
                    partition,
                    CallableIndexKeyFact::Constant(value.clone()),
                    CallableIndexAnchorFact::InFlightSelection(point),
                )
            } else {
                vec![partition.clone()]
            }
        };
        for partition in live_alternatives {
            if let Some(live) = &live {
                let mut next = partition.clone();
                let cell = next
                    .representatives
                    .iter()
                    .find(|rep| rep.key == *live)
                    .expect("live index representative")
                    .cell
                    .clone();
                next.representatives.push(CallableIndexRepresentativeFact {
                    key: evaluated.clone(),
                    anchors: vec![CallableIndexAnchorFact::InFlightSelection(point)],
                    cell,
                });
                next.relations.push(CallableIndexEqualityFact {
                    left: evaluated.clone(),
                    right: live.clone(),
                    relation: CallableArrayIndexRelation::Equal,
                });
                if normalize_indexed_partition(&mut next) {
                    alternatives.push(next);
                }
            } else {
                if let Some(value) = &constant {
                    let known = CallableIndexKeyFact::Constant(value.clone());
                    let mut next = partition.clone();
                    let cell = next
                        .representatives
                        .iter()
                        .find(|representative| representative.key == known)
                        .expect("constant borrow representative")
                        .cell
                        .clone();
                    next.representatives.push(CallableIndexRepresentativeFact {
                        key: evaluated.clone(),
                        anchors: vec![CallableIndexAnchorFact::InFlightSelection(point)],
                        cell,
                    });
                    next.relations.push(CallableIndexEqualityFact {
                        left: evaluated.clone(),
                        right: known,
                        relation: CallableArrayIndexRelation::Equal,
                    });
                    if normalize_indexed_partition(&mut next) {
                        alternatives.push(next);
                    }
                } else {
                    alternatives.extend(select_indexed_partition(
                        &partition,
                        evaluated.clone(),
                        CallableIndexAnchorFact::InFlightSelection(point),
                    ));
                }
            }
        }
    }
    family.partitions = alternatives;
    normalize_indexed_partitions(family);
}

fn fold_indexed_representative(family: &mut CallableArrayFamilyFact, key: &CallableIndexKeyFact) {
    for partition in &mut family.partitions {
        if let Some(position) = partition
            .representatives
            .iter()
            .position(|rep| &rep.key == key)
        {
            let representative = partition.representatives.remove(position);
            let has_equal_anchor = partition.relations.iter().any(|relation| {
                relation.relation == CallableArrayIndexRelation::Equal
                    && ((&relation.left == key
                        && partition
                            .representatives
                            .iter()
                            .any(|rep| rep.key == relation.right))
                        || (&relation.right == key
                            && partition
                                .representatives
                                .iter()
                                .any(|rep| rep.key == relation.left)))
            });
            if !has_equal_anchor {
                for case in representative.cell.alternatives {
                    if !partition.remainder.alternatives.contains(&case) {
                        partition.remainder.alternatives.push(case);
                    }
                }
            }
            partition
                .relations
                .retain(|relation| &relation.left != key && &relation.right != key);
        }
    }
    normalize_indexed_partitions(family);
}

fn pin_indexed_reference_loan(
    state: &mut ReferenceEdgeState,
    points: &[ReferenceFlowPoint],
    id: CfgPointId,
    loan: &ReferenceLoanId,
    indices: &[ReferenceBorrowIndexInput],
) {
    let Some(ReferencePlaceProjection::Index(index)) = loan.origin_place.projections.last() else {
        return;
    };
    let index_span = expression_span(index);
    let input = indices
        .iter()
        .find(|input| input.projection + 1 == loan.origin_place.projections.len())
        .expect("actual borrow index input");
    let (live, constant) = match index {
        ScalarExpression::Name { name, .. } => {
            let CfgPointKind::Read { binding } = &points[input.value_point.0].kind else {
                panic!("evaluated checked borrow index")
            };
            let binding = binding.clone();
            let live = CallableIndexKeyFact::LivePlace(ReferencePlaceId {
                source: binding.source.clone(),
                function_span: binding.function_span,
                declaration_span: binding.declaration_span,
                block_span: binding.block_span,
                binding: binding.clone(),
                projections: Vec::new(),
                place: ScalarPlace::Name {
                    name: name.clone(),
                    span: binding.declaration_span,
                },
            });
            (Some(live), state.index_values.get(&binding).cloned())
        }
        ScalarExpression::Integer { value, .. } => (None, Some(value.clone())),
        _ => (None, None),
    };
    let occurrence = SourceSpan::new(loan.origin_place.source.clone(), index_span);
    let key = CallableIndexKeyFact::Evaluated {
        point: input.value_point,
        occurrence,
        function_span: loan.origin_place.function_span,
    };
    if let Some(backings) = state.backing_bindings.get_mut(&loan.origin_place.binding) {
        for backing in backings {
            if backing.kind != RuntimeBackingKind::OwnedAllocation {
                continue;
            }
            for family in &mut backing.callable_family {
                evaluate_indexed_family(family, key.clone(), live.clone(), constant.clone(), id);
                for partition in &mut family.partitions {
                    let representative = partition
                        .representatives
                        .iter_mut()
                        .find(|representative| representative.key == key)
                        .expect("checked loan selected index");
                    representative.anchors = vec![CallableIndexAnchorFact::ActiveLoan {
                        loan: loan.clone(),
                        backing: backing.value.clone(),
                        owner: backing.owner.clone(),
                        length: backing.length.clone(),
                        origin: SourceSpan::new(loan.creation_source.clone(), loan.creation_span),
                    }];
                }
                normalize_indexed_partitions(family);
            }
        }
    }
}

fn expire_indexed_reference_loans(
    state: &mut ReferenceEdgeState,
    points: &[ReferenceFlowPoint],
    donor_uses: &BTreeMap<CfgPointId, ReferenceBindingId>,
    facts: &[ReferenceOrigin],
    from: CfgPointId,
    successor: CfgPointId,
) {
    let active = |loan: &ReferenceLoanId| {
        state.bindings.iter().any(|(binding, origins)| {
            reference_read_reachable(points, donor_uses, from, binding)
                && origins.iter().any(|origin| {
                    matches!(&facts[origin.0],
                    ReferenceOrigin::Fresh { loan: held, .. }
                    | ReferenceOrigin::BorrowedFrom { loan: held, .. } if held == loan)
                })
        }) || matches!(&points[successor.0].kind, CfgPointKind::Bind { .. })
            && state.value.as_ref().is_some_and(|origins| {
                origins.iter().any(|origin| {
                    matches!(&facts[origin.0],
                ReferenceOrigin::Fresh { loan: held, .. }
                | ReferenceOrigin::BorrowedFrom { loan: held, .. } if held == loan)
                })
            })
    };
    let mut expired = Vec::new();
    for backings in state.backing_bindings.values() {
        for family in backings.iter().flat_map(|backing| &backing.callable_family) {
            for partition in &family.partitions {
                for representative in &partition.representatives {
                    if representative.anchors.iter().any(|anchor| {
                        matches!(anchor,
                        CallableIndexAnchorFact::ActiveLoan { loan, .. } if !active(loan))
                    }) && !expired.contains(&representative.key)
                    {
                        expired.push(representative.key.clone());
                    }
                }
            }
        }
    }
    for backings in state.backing_bindings.values_mut() {
        for family in backings
            .iter_mut()
            .flat_map(|backing| &mut backing.callable_family)
        {
            for key in &expired {
                fold_indexed_representative(family, key);
            }
        }
    }
}

fn indexed_loan_cells(
    state: &ReferenceEdgeState,
    loan: &ReferenceLoanId,
) -> Vec<(
    CallableArraySelectedHolderFact,
    Vec<CallableIndexEqualityFact>,
    CallableCellCaseFact,
    CallableIndexKeyFact,
)> {
    let mut selected = Vec::new();
    if let Some(backings) = state.backing_bindings.get(&loan.origin_place.binding) {
        for backing in backings {
            for family in &backing.callable_family {
                for partition in &family.partitions {
                    for representative in &partition.representatives {
                        let anchor =
                            representative
                                .anchors
                                .iter()
                                .find_map(|anchor| match anchor {
                                    CallableIndexAnchorFact::ActiveLoan {
                                        loan: active,
                                        backing: value,
                                        owner,
                                        length,
                                        ..
                                    } if active == loan
                                        && value == &backing.value
                                        && owner == &backing.owner
                                        && length == &backing.length =>
                                    {
                                        Some((value, owner, length))
                                    }
                                    _ => None,
                                });
                        let Some((value, owner, length)) = anchor else {
                            continue;
                        };
                        let CallableIndexKeyFact::Evaluated {
                            point, occurrence, ..
                        } = &representative.key
                        else {
                            continue;
                        };
                        let holder = CallableArraySelectedHolderFact {
                            value: value.clone(),
                            owner: owner.clone(),
                            length: length.clone(),
                            index_point: *point,
                            occurrence: occurrence.clone(),
                            place: loan.origin_place.clone(),
                        };
                        for case in &representative.cell.alternatives {
                            selected.push((
                                holder.clone(),
                                partition.relations.clone(),
                                case.clone(),
                                representative.key.clone(),
                            ));
                        }
                    }
                }
            }
        }
    }
    selected
}

fn selected_previous_element(
    case: &CallableCellCaseFact,
    partition: &CallableIndexedPartitionFact,
    key: &CallableIndexKeyFact,
    place: &ReferencePlaceId,
    index_point: CfgPointId,
    occurrence: &SourceSpan,
) -> CallableArrayElementFact {
    let CallableCellCaseFact::Initialized {
        environment_origin,
        holder_aliases,
        loans,
        ..
    } = case
    else {
        unreachable!()
    };
    let constant = partition.representatives.iter().find_map(|representative| {
        let CallableIndexKeyFact::Constant(value) = &representative.key else {
            return None;
        };
        partition
            .relations
            .iter()
            .any(|relation| {
                relation.relation == CallableArrayIndexRelation::Equal
                    && ((&relation.left == key && relation.right == representative.key)
                        || (&relation.right == key && relation.left == representative.key))
            })
            .then(|| value.clone())
    });
    let index = match constant {
        Some(value) => CallableArrayIndexFact::Constant(value),
        None => CallableArrayIndexFact::Selected {
            point: index_point,
            occurrence: occurrence.clone(),
            function_span: place.function_span,
        },
    };
    CallableArrayElementFact {
        place: place.clone(),
        index,
        environment: environment_origin.clone(),
        instances: CallableArrayEnvironmentInstances::PerEvaluatedIndex,
        aliases: holder_aliases.clone(),
        loans: loans.clone(),
    }
}

#[cfg(test)]
mod indexed_partition_tests {
    use super::*;

    fn source() -> SourceIdentity {
        SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        )
    }

    fn live(name: &str, position: u32) -> CallableIndexKeyFact {
        let span = ByteSpan::new(position, position + name.len() as u32);
        let binding = ReferenceBindingId {
            source: source(),
            function_span: ByteSpan::new(0, 100),
            declaration_span: span,
            block_span: ByteSpan::new(0, 100),
            kind: ReferenceBindingKind::Declared,
        };
        CallableIndexKeyFact::LivePlace(ReferencePlaceId {
            source: source(),
            function_span: binding.function_span,
            declaration_span: span,
            block_span: binding.block_span,
            binding,
            projections: Vec::new(),
            place: ScalarPlace::Name {
                name: name.to_owned(),
                span,
            },
        })
    }

    fn case(index: usize) -> CallableCellCaseFact {
        let span = ByteSpan::new(20 + index as u32, 21 + index as u32);
        let owner = ReferenceBindingId {
            source: source(),
            function_span: ByteSpan::new(0, 100),
            declaration_span: span,
            block_span: ByteSpan::new(0, 100),
            kind: ReferenceBindingKind::Declared,
        };
        CallableCellCaseFact::Initialized {
            environment_origin: CallableEnvironmentId {
                literal: SourceSpan::new(source(), span),
                index,
            },
            instance: CallableEnvironmentInstanceClassFact::FreshPerCell,
            holder_aliases: Vec::new(),
            backing: RuntimeValueId {
                source: source(),
                function_span: ByteSpan::new(0, 100),
                creation_span: span,
                position: RuntimeValuePosition::Local,
            },
            owner,
            capture_paths: Vec::new(),
            loans: BTreeSet::new(),
            moved: false,
            origin: SourceSpan::new(source(), span),
        }
    }

    fn joint_partition(left: usize, right: usize) -> CallableIndexedPartitionFact {
        let common = case(0);
        let cells = [left, right].map(|index| {
            let mut value = case(index);
            match (&mut value, &common) {
                (
                    CallableCellCaseFact::Initialized { backing, owner, .. },
                    CallableCellCaseFact::Initialized {
                        backing: common_backing,
                        owner: common_owner,
                        ..
                    },
                ) => {
                    *backing = common_backing.clone();
                    *owner = common_owner.clone();
                }
                _ => unreachable!(),
            }
            value
        });
        CallableIndexedPartitionFact {
            representatives: cells
                .into_iter()
                .enumerate()
                .map(|(index, value)| CallableIndexRepresentativeFact {
                    key: CallableIndexKeyFact::Constant(index.into()),
                    anchors: Vec::new(),
                    cell: CallableCellDomainFact {
                        alternatives: vec![value],
                    },
                })
                .collect(),
            relations: Vec::new(),
            remainder: CallableCellDomainFact {
                alternatives: vec![CallableCellCaseFact::Uninitialized],
            },
        }
    }

    #[test]
    fn compositional_assignment_result_support_family_join_loses_joint_cell_tuple() {
        let first = joint_partition(0, 1);
        let second = joint_partition(1, 0);
        let mut joined = first.clone();
        assert!(merge_indexed_partition(&mut joined, &second));
        assert!(normalize_indexed_partition(&mut joined));
        let unsupported = joint_partition(0, 0);
        assert_ne!(unsupported, first);
        assert_ne!(unsupported, second);
        for (selected, merged) in unsupported
            .representatives
            .iter()
            .zip(&joined.representatives)
        {
            assert_eq!(selected.key, merged.key);
            assert!(merged
                .cell
                .alternatives
                .contains(&selected.cell.alternatives[0]));
        }
        assert_eq!(joined.representatives[0].cell.alternatives.len(), 2);
        assert_eq!(joined.representatives[1].cell.alternatives.len(), 2);
        assert!(!merge_indexed_partition(&mut joined, &unsupported));
    }

    #[test]
    fn compositional_assignment_result_support_family_join_distinct_joint_inputs_same_valuation() {
        let mut crossed = joint_partition(0, 1);
        let mut diagonal = joint_partition(0, 0);
        assert!(merge_indexed_partition(
            &mut crossed,
            &joint_partition(1, 0)
        ));
        assert!(merge_indexed_partition(
            &mut diagonal,
            &joint_partition(1, 1)
        ));
        assert!(normalize_indexed_partition(&mut crossed));
        assert!(normalize_indexed_partition(&mut diagonal));
        assert_eq!(crossed, diagonal);
    }

    fn whole_state(left: usize, right: usize) -> ReferenceEdgeState {
        let CallableCellCaseFact::Initialized {
            backing,
            owner,
            origin,
            ..
        } = case(0)
        else {
            unreachable!()
        };
        let CallableIndexKeyFact::LivePlace(destination) = live("destination", 4) else {
            unreachable!()
        };
        let destination_id = AssignmentDestinationId {
            operation: AssignmentOperationId {
                producer: ProducerCfgOwnerId::Module(source()),
                occurrence: ScalarAssignmentOccurrence {
                    source_span: SourceSpan::new(source(), destination.binding.declaration_span),
                    output_projection: Vec::new(),
                },
            },
            target: 0,
        };
        let frame = FrameVariableRef {
            owner: destination_id.operation.producer.clone(),
            variable: SupportedInstanceVariable(0),
        };
        let mut state = ReferenceEdgeState {
            assignment_destinations: BTreeMap::from([(
                CfgPointId(1),
                vec![FrozenAssignmentDestination {
                    selection: FrozenSelectionOccurrence {
                        application: AssignmentApplicationRef {
                            frame: frame.clone(),
                            operation: destination_id.operation.clone(),
                        },
                        destination: destination_id.clone(),
                    },
                    destination: destination_id,
                    point: CfgPointId(1),
                    place: destination,
                    access: FrozenAssignmentAccess::Direct,
                }],
            )]),
            assignment_saved: BTreeMap::new(),
            point_values: vec![Some(BTreeSet::from([ReferenceOriginId(0)]))],
            backing_value: vec![RuntimeBackingFact {
                value: backing,
                owner: owner.clone(),
                kind: RuntimeBackingKind::OwnedAllocation,
                origin,
                length: RuntimeArrayLengthFact::Parameter { binding: owner },
                callable_family: vec![CallableArrayFamilyFact {
                    element: ScalarType::Callable {
                        outputs: ScalarOutputSequence {
                            outputs: Vec::new(),
                            span: ByteSpan::new(20, 21),
                        },
                        parameters: Vec::new(),
                    },
                    elements: Vec::new(),
                    index_relations: Vec::new(),
                    unselected: CallableArrayUnselectedFact {
                        selected: Vec::new(),
                    },
                    partitions: vec![joint_partition(left, right)],
                }],
            }],
            ..ReferenceEdgeState::new(frame)
        };
        state.backing_bindings.insert(
            state.backing_value[0].owner.clone(),
            state.backing_value.clone(),
        );
        state
    }

    fn admit_distinct_pair(first: &ReferenceEdgeState, second: &ReferenceEdgeState) {
        let first_before = first.clone();
        let second_before = second.clone();
        let mut states = Vec::new();
        assert!(reference_state::admit(&mut states, first));
        assert!(reference_state::admit(&mut states, second));
        assert!(states == vec![first_before.clone(), second_before.clone()]);
        let before_duplicate = states.clone();
        assert!(!reference_state::admit(&mut states, first));
        assert!(!reference_state::admit(&mut states, second));
        assert!(states == before_duplicate);
        assert!(*first == first_before);
        assert!(*second == second_before);
    }

    #[test]
    fn whole_state_admission_support_crossed_tuple() {
        let first = whole_state(0, 1);
        let second = whole_state(1, 0);
        admit_distinct_pair(&first, &second);
        let mut states = Vec::new();
        assert!(reference_state::admit(&mut states, &first));
        assert!(reference_state::admit(&mut states, &second));
        assert_eq!(states.len(), 2);
        assert!(!states.contains(&whole_state(0, 0)));
        assert!(!states.contains(&whole_state(1, 1)));
    }

    #[test]
    fn whole_state_admission_support_diagonal_tuple() {
        let first = whole_state(0, 0);
        let second = whole_state(1, 1);
        admit_distinct_pair(&first, &second);
        let mut states = Vec::new();
        assert!(reference_state::admit(&mut states, &first));
        assert!(reference_state::admit(&mut states, &second));
        assert_eq!(states.len(), 2);
        assert!(!states.contains(&whole_state(0, 1)));
        assert!(!states.contains(&whole_state(1, 0)));
    }

    #[test]
    fn whole_state_admission_support_distinct_relations() {
        let crossed = [whole_state(0, 1), whole_state(1, 0)];
        let diagonal = [whole_state(0, 0), whole_state(1, 1)];
        let mut crossed_states = Vec::new();
        let mut diagonal_states = Vec::new();
        for state in &crossed {
            assert!(reference_state::admit(&mut crossed_states, state));
        }
        for state in &diagonal {
            assert!(reference_state::admit(&mut diagonal_states, state));
        }
        assert!(crossed_states == crossed);
        assert!(diagonal_states == diagonal);
        assert!(crossed_states != diagonal_states);
        assert_eq!(crossed_states.len(), 2);
        assert_eq!(diagonal_states.len(), 2);
    }

    #[test]
    fn whole_state_admission_support_duplicate_preserves_complete_content() {
        let incoming = whole_state(0, 1);
        let incoming_before = incoming.clone();
        let mut states = Vec::new();
        assert!(reference_state::admit(&mut states, &incoming));
        assert!(states == vec![incoming_before.clone()]);
        let before = states.clone();
        assert!(!reference_state::admit(&mut states, &incoming));
        assert!(states == before);
        assert!(incoming == incoming_before);
    }

    #[test]
    fn whole_state_admission_support_frozen_assignment_destinations() {
        let first = whole_state(0, 1);
        let mut second = first.clone();
        let CallableIndexKeyFact::LivePlace(destination) = live("other", 8) else {
            unreachable!()
        };
        let mut frozen = first.assignment_destinations[&CfgPointId(1)][0].clone();
        frozen.place = destination;
        assert_eq!(
            frozen.destination,
            first.assignment_destinations[&CfgPointId(1)][0].destination
        );
        assert_eq!(
            frozen.point,
            first.assignment_destinations[&CfgPointId(1)][0].point
        );
        assert_eq!(
            frozen.access,
            first.assignment_destinations[&CfgPointId(1)][0].access
        );
        second
            .assignment_destinations
            .insert(CfgPointId(1), vec![frozen]);
        assert_ne!(
            first.assignment_destinations,
            second.assignment_destinations
        );
        admit_distinct_pair(&first, &second);
    }

    #[test]
    fn whole_state_admission_support_saved_point_values() {
        let first = whole_state(0, 1);
        let mut second = first.clone();
        second.point_values[0] = Some(BTreeSet::from([ReferenceOriginId(1)]));
        assert_ne!(first.point_values, second.point_values);
        admit_distinct_pair(&first, &second);
    }

    #[test]
    fn whole_state_admission_support_runtime_value_lineage() {
        let mut first = whole_state(0, 1);
        let lineage = first.backing_value[0].value.clone();
        first.backing_value[0].value.position = RuntimeValuePosition::CapturedCall {
            call: SourceSpan::new(source(), ByteSpan::new(30, 35)),
            output: 0,
            lineage: Box::new(lineage.clone()),
        };
        let mut second = first.clone();
        let mut different_lineage = lineage;
        different_lineage.position = RuntimeValuePosition::Parameter {
            declaration_span: ByteSpan::new(40, 45),
        };
        let RuntimeValuePosition::CapturedCall { lineage, .. } =
            &mut second.backing_value[0].value.position
        else {
            unreachable!()
        };
        *lineage = Box::new(different_lineage);
        assert_ne!(first.backing_value[0].value, second.backing_value[0].value);
        admit_distinct_pair(&first, &second);
    }

    #[test]
    fn whole_state_admission_support_runtime_value_source_qualifiers() {
        let first = whole_state(0, 1);
        let mut second = first.clone();
        second.backing_value[0].value.source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/other.w".into(),
            "r1".into(),
        );
        assert_ne!(first.backing_value[0].value, second.backing_value[0].value);
        admit_distinct_pair(&first, &second);
    }

    #[test]
    fn indexed_partition_closes_equalities_prunes_contradictions_and_keeps_whole_cells() {
        let zero = CallableIndexKeyFact::Constant(0.into());
        let one = CallableIndexKeyFact::Constant(1.into());
        let i = live("i", 4);
        let j = live("j", 6);
        let mut partition = CallableIndexedPartitionFact {
            representatives: vec![
                CallableIndexRepresentativeFact {
                    key: zero.clone(),
                    anchors: Vec::new(),
                    cell: CallableCellDomainFact {
                        alternatives: vec![case(0)],
                    },
                },
                CallableIndexRepresentativeFact {
                    key: one.clone(),
                    anchors: Vec::new(),
                    cell: CallableCellDomainFact {
                        alternatives: vec![case(1)],
                    },
                },
                CallableIndexRepresentativeFact {
                    key: i.clone(),
                    anchors: Vec::new(),
                    cell: CallableCellDomainFact {
                        alternatives: vec![case(2)],
                    },
                },
            ],
            relations: vec![CallableIndexEqualityFact {
                left: i.clone(),
                right: zero.clone(),
                relation: CallableArrayIndexRelation::Equal,
            }],
            remainder: CallableCellDomainFact {
                alternatives: vec![CallableCellCaseFact::Uninitialized],
            },
        };
        assert!(normalize_indexed_partition(&mut partition));
        let zero_cell = &partition
            .representatives
            .iter()
            .find(|rep| rep.key == zero)
            .expect("zero")
            .cell;
        assert_eq!(zero_cell.alternatives.len(), 2);
        assert!(
            zero_cell.alternatives.contains(&case(0)) && zero_cell.alternatives.contains(&case(2))
        );
        let branches = select_indexed_partition(
            &partition,
            j.clone(),
            CallableIndexAnchorFact::ValuePlace(match &j {
                CallableIndexKeyFact::LivePlace(place) => place.clone(),
                _ => unreachable!(),
            }),
        );
        assert_eq!(branches.len(), 3);
        let relation = |partition: &CallableIndexedPartitionFact,
                        left: &CallableIndexKeyFact,
                        right: &CallableIndexKeyFact| {
            partition
                .relations
                .iter()
                .find(|fact| {
                    (&fact.left == left && &fact.right == right)
                        || (&fact.left == right && &fact.right == left)
                })
                .map(|fact| fact.relation.clone())
        };
        assert!(branches.iter().any(|branch| relation(branch, &j, &zero)
            == Some(CallableArrayIndexRelation::Equal)
            && relation(branch, &j, &i) == Some(CallableArrayIndexRelation::Equal)
            && relation(branch, &j, &one) == Some(CallableArrayIndexRelation::Distinct)));
        assert!(branches.iter().any(|branch| relation(branch, &j, &one)
            == Some(CallableArrayIndexRelation::Equal)
            && relation(branch, &j, &i) == Some(CallableArrayIndexRelation::Distinct)));
        assert!(branches.iter().any(|branch| relation(branch, &j, &one)
            == Some(CallableArrayIndexRelation::Distinct)
            && relation(branch, &j, &i) == Some(CallableArrayIndexRelation::Distinct)));
        let mut contradictory = branches
            .iter()
            .find(|branch| relation(branch, &j, &zero) == Some(CallableArrayIndexRelation::Equal))
            .expect("j equals zero")
            .clone();
        contradictory.relations.push(CallableIndexEqualityFact {
            left: j,
            right: i.clone(),
            relation: CallableArrayIndexRelation::Distinct,
        });
        assert!(!normalize_indexed_partition(&mut contradictory));
        let restored: CallableIndexedPartitionFact =
            serde_json::from_str(&serde_json::to_string(&partition).unwrap()).unwrap();
        assert_eq!(restored, partition);

        let k = live("k", 8);
        let unknown = CallableIndexedPartitionFact {
            representatives: vec![
                CallableIndexRepresentativeFact {
                    key: i.clone(),
                    anchors: Vec::new(),
                    cell: CallableCellDomainFact {
                        alternatives: vec![case(0)],
                    },
                },
                CallableIndexRepresentativeFact {
                    key: k.clone(),
                    anchors: Vec::new(),
                    cell: CallableCellDomainFact {
                        alternatives: vec![case(1)],
                    },
                },
            ],
            relations: Vec::new(),
            remainder: CallableCellDomainFact {
                alternatives: vec![CallableCellCaseFact::Uninitialized],
            },
        };
        let other = live("other", 10);
        let unknown_branches = select_indexed_partition(
            &unknown,
            other.clone(),
            CallableIndexAnchorFact::ValuePlace(match &other {
                CallableIndexKeyFact::LivePlace(place) => place.clone(),
                _ => unreachable!(),
            }),
        );
        assert_eq!(unknown_branches.len(), 4);
        assert!(unknown_branches
            .iter()
            .any(
                |branch| relation(branch, &other, &i) == Some(CallableArrayIndexRelation::Equal)
                    && relation(branch, &other, &k) == Some(CallableArrayIndexRelation::Distinct)
            ));
        assert!(unknown_branches.iter().any(|branch| {
            relation(branch, &other, &i) == Some(CallableArrayIndexRelation::Equal)
                && relation(branch, &other, &k) == Some(CallableArrayIndexRelation::Equal)
                && branch
                    .representatives
                    .iter()
                    .find(|rep| rep.key == other)
                    .expect("selected cell")
                    .cell
                    .alternatives
                    .contains(&case(1))
        }));
        let mut transitive = unknown;
        transitive.relations.push(CallableIndexEqualityFact {
            left: i.clone(),
            right: k.clone(),
            relation: CallableArrayIndexRelation::Equal,
        });
        transitive.relations.push(CallableIndexEqualityFact {
            left: k,
            right: zero,
            relation: CallableArrayIndexRelation::Distinct,
        });
        transitive
            .representatives
            .push(CallableIndexRepresentativeFact {
                key: CallableIndexKeyFact::Constant(0.into()),
                anchors: Vec::new(),
                cell: CallableCellDomainFact {
                    alternatives: vec![case(2)],
                },
            });
        assert!(normalize_indexed_partition(&mut transitive));
        assert_eq!(
            relation(&transitive, &i, &CallableIndexKeyFact::Constant(0.into())),
            Some(CallableArrayIndexRelation::Distinct)
        );
        let zero_selection = select_indexed_partition(
            &transitive,
            CallableIndexKeyFact::Constant(0.into()),
            CallableIndexAnchorFact::ValuePlace(match &i {
                CallableIndexKeyFact::LivePlace(place) => place.clone(),
                _ => unreachable!(),
            }),
        );
        assert_eq!(zero_selection.len(), 1);
        assert_eq!(
            relation(
                &zero_selection[0],
                &i,
                &CallableIndexKeyFact::Constant(0.into())
            ),
            Some(CallableArrayIndexRelation::Distinct)
        );
    }
}

fn transfer_array_family(
    fact: &mut RuntimeBackingFact,
    source: &ReferenceBindingId,
    target: &ReferenceBindingId,
    copy: bool,
) {
    for family in &mut fact.callable_family {
        for partition in &mut family.partitions {
            for representative in &mut partition.representatives {
                if let CallableIndexKeyFact::LivePlace(place) = &mut representative.key {
                    if place.binding == *source {
                        place.binding = target.clone();
                    }
                }
                for case in &mut representative.cell.alternatives {
                    if let CallableCellCaseFact::Initialized { owner, backing, .. } = case {
                        if *owner == *source {
                            *owner = target.clone();
                        }
                        *backing = fact.value.clone();
                    }
                }
            }
            for case in &mut partition.remainder.alternatives {
                if let CallableCellCaseFact::Initialized { owner, backing, .. } = case {
                    if *owner == *source {
                        *owner = target.clone();
                    }
                    *backing = fact.value.clone();
                }
            }
        }
        for element in &mut family.elements {
            if element.place.binding == *source {
                element.place.binding = target.clone();
                element.place.source = target.source.clone();
                element.place.function_span = target.function_span;
                element.place.declaration_span = target.declaration_span;
                element.place.block_span = target.block_span;
            }
            let mut aliases = Vec::new();
            for alias in &element.aliases {
                if let CallableHolderPlace::Projection(place) = alias {
                    if place.binding == *source {
                        if copy {
                            aliases.push(alias.clone());
                        }
                        let mut next = place.clone();
                        next.binding = target.clone();
                        next.source = target.source.clone();
                        next.function_span = target.function_span;
                        next.declaration_span = target.declaration_span;
                        next.block_span = target.block_span;
                        aliases.push(CallableHolderPlace::Projection(next));
                        continue;
                    }
                }
                aliases.push(alias.clone());
            }
            aliases.sort_by(holder_place_cmp);
            aliases.dedup_by(|left, right| holder_place_same(left, right));
            element.aliases = aliases;
        }
    }
}

fn contains_projected_runtime_array(ty: &ScalarType, structs: &[ScalarStruct]) -> bool {
    let ScalarType::Struct(structure) = ty else {
        return false;
    };
    structs
        .iter()
        .find(|item| item.id == *structure)
        .expect("resolved struct type")
        .fields
        .iter()
        .any(|field| {
            matches!(field.ty, ScalarType::RuntimeArray { .. })
                || contains_projected_runtime_array(&field.ty, structs)
        })
}

fn remove_holder(
    relations: &mut Vec<CallableHolderRelation>,
    holder: &CallableHolderPlace,
    span: &SourceSpan,
    actions: &mut Vec<CallableHolderAction>,
    provenance: &mut HolderProvenance,
) {
    for relation in relations.iter_mut() {
        if let Some(index) = relation
            .holders
            .iter()
            .position(|known| holder_place_same(known, holder))
        {
            let previous = relation.holders.remove(index);
            actions.push(CallableHolderAction::Drop {
                environment: relation.environment.clone(),
                holder: previous,
                span: span.clone(),
            });
        }
    }
    provenance.remove_holder(holder);
    normalize_holder_relations(relations);
}

fn transfer_holder(
    relations: &mut Vec<CallableHolderRelation>,
    source: &CallableHolderPlace,
    target: CallableHolderPlace,
    moved: bool,
    span: &SourceSpan,
    actions: &mut Vec<CallableHolderAction>,
    point: CfgPointId,
    provenance: &mut HolderProvenance,
) {
    let mut forwarded = Vec::new();
    for relation in relations.iter_mut() {
        let Some(previous) = relation
            .holders
            .iter()
            .find(|known| holder_place_same(known, source))
            .cloned()
        else {
            continue;
        };
        let is_move = moved
            || matches!(
                source,
                CallableHolderPlace::Temporary(_)
                    | CallableHolderPlace::Projection(ReferencePlaceId {
                        binding: ReferenceBindingId {
                            kind: ReferenceBindingKind::CallOutput { .. },
                            ..
                        },
                        ..
                    })
            );
        if is_move {
            relation
                .holders
                .retain(|holder| !holder_place_same(holder, &previous));
        }
        let action = if is_move {
            CallableHolderAction::Move {
                environment: relation.environment.clone(),
                from: previous.clone(),
                to: target.clone(),
                span: span.clone(),
            }
        } else {
            CallableHolderAction::Retain {
                environment: relation.environment.clone(),
                from: previous.clone(),
                to: target.clone(),
                span: span.clone(),
            }
        };
        actions.push(action.clone());
        relation.holders.push(target.clone());
        forwarded.push((relation.environment.clone(), is_move, previous, action));
    }
    for (environment, is_move, previous, action) in forwarded {
        let previous_chain = provenance.lookup(&environment, &previous).cloned();
        if is_move {
            provenance.remove(&environment, &previous);
        }
        if let Some(chain) = previous_chain {
            provenance.insert(
                environment.clone(),
                target.clone(),
                HolderProducerRef::Forward {
                    point,
                    environment,
                    action,
                    previous: Box::new(chain),
                },
            );
        }
    }
    normalize_holder_relations(relations);
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
    donor_uses: &BTreeMap<CfgPointId, ReferenceBindingId>,
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
        if donor_uses.get(&id) == Some(binding) { return true; }
        if let ReferenceBindingKind::AssignmentPrevious { snapshot, .. } = binding.kind {
            if matches!(&points[id.0].kind, CfgPointKind::AssignmentResult { snapshot_point, .. } if *snapshot_point == snapshot)
            {
                return true;
            }
        }
        if let CfgPointKind::AssignmentSnapshot {
            destination_point,
            observed: true,
            ..
        } = &points[id.0].kind
        {
            if matches!(&points[destination_point.0].kind, CfgPointKind::AssignmentDestination { storage: EvaluatedDestinationStorage::Direct(place), .. } if &place.binding == binding)
            {
                return true;
            }
        }
        if matches!(&points[id.0].kind, CfgPointKind::ReadPlace { pointer_point: Some(pointer), .. }
            if matches!(&points[pointer.0].kind, CfgPointKind::Read { binding: read } if read == binding))
        {
            return true;
        }
        if matches!(&points[id.0].kind, CfgPointKind::Capture { captures, .. } if captures.iter().any(|capture| &capture.binding == binding))
        {
            return true;
        }
        if matches!(&points[id.0].kind, CfgPointKind::Call { target: ResolvedCallTarget::LocalCallable(callee), .. } if callee == binding)
            || matches!(&points[id.0].kind, CfgPointKind::Call { target: ResolvedCallTarget::CallableField { receiver_place, .. }, .. } if &receiver_place.binding == binding)
            || matches!(&points[id.0].kind, CfgPointKind::ReadPlace { candidates, .. } if candidates.iter().any(|place| &place.binding == binding))
        {
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
}

fn indexed_loan_borrows_place(
    state: &ReferenceEdgeState,
    loan: &ReferenceLoanId,
    place: &ReferencePlaceId,
) -> bool {
    if !loan_borrows_place(loan, place) {
        return false;
    }
    let Some(ReferencePlaceProjection::Index(target_index)) = place.projections.last() else {
        return true;
    };
    let Some(ReferencePlaceProjection::Index(_)) = loan.origin_place.projections.last() else {
        return true;
    };
    let selected = match target_index {
        ScalarExpression::Integer { value, .. } => CallableIndexKeyFact::Constant(value.clone()),
        _ => {
            let Some((point, selected_place)) = &state.selected_element else {
                return true;
            };
            let Some(ReferencePlaceProjection::Index(expression)) =
                selected_place.projections.last()
            else {
                return true;
            };
            CallableIndexKeyFact::Evaluated {
                point: *point,
                occurrence: SourceSpan::new(
                    selected_place.source.clone(),
                    expression_span(expression),
                ),
                function_span: selected_place.function_span,
            }
        }
    };
    let Some(backings) = state.backing_bindings.get(&place.binding) else {
        return true;
    };
    let mut found = false;
    for partition in backings
        .iter()
        .flat_map(|backing| &backing.callable_family)
        .flat_map(|family| &family.partitions)
    {
        let Some(pinned) = partition.representatives.iter().find(|representative| {
            representative.anchors.iter().any(|anchor| {
                matches!(anchor,
                CallableIndexAnchorFact::ActiveLoan { loan: active, .. } if active == loan)
            })
        }) else {
            return true;
        };
        found = true;
        let distinct = partition.relations.iter().any(|relation| {
            relation.relation == CallableArrayIndexRelation::Distinct
                && ((relation.left == pinned.key && relation.right == selected)
                    || (relation.right == pinned.key && relation.left == selected))
        }) || match &selected {
            CallableIndexKeyFact::Constant(value) => partition.representatives.iter().any(|representative| {
                matches!(&representative.key, CallableIndexKeyFact::Constant(other) if other != value)
                    && partition.relations.iter().any(|relation| relation.relation == CallableArrayIndexRelation::Equal
                        && ((relation.left == pinned.key && relation.right == representative.key)
                            || (relation.right == pinned.key && relation.left == representative.key)))
            }),
            _ => false,
        };
        if !distinct {
            return true;
        }
    }
    !found
}

fn callable_field_read_reachable(
    points: &[ReferenceFlowPoint],
    donor_uses: &BTreeMap<CfgPointId, ReferenceBindingId>,
    from: CfgPointId,
    place: &ReferencePlaceId,
) -> bool {
    let mut pending = points[from.0].successors.clone();
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id.0) {
            continue;
        }
        let point = &points[id.0];
        if donor_uses.get(&id) == Some(&place.binding) { return true; }
        if matches!(&point.kind, CfgPointKind::ReadPlace { candidates, .. } if candidates.iter().any(|read| reference_places_overlap(read, place)))
        {
            return true;
        }
        if let CfgPointKind::Call {
            target:
                ResolvedCallTarget::CallableField {
                    receiver_place,
                    field,
                },
            ..
        } = &point.kind
        {
            let mut target = receiver_place.clone();
            target.projections.push(ReferencePlaceProjection::Field(
                ScalarFieldReference::Resolved(field.clone()),
            ));
            if reference_places_overlap(&target, place) {
                return true;
            }
        }
        if matches!(&point.kind, CfgPointKind::Capture { captures, .. } if captures.iter().any(|capture| capture.binding == place.binding && reference_places_overlap(&ReferencePlaceId { source: capture.binding.source.clone(), function_span: capture.binding.function_span, declaration_span: capture.binding.declaration_span, block_span: capture.binding.block_span, binding: capture.binding.clone(), projections: Vec::new(), place: capture.place.clone() }, place)))
        {
            return true;
        }
        if matches!(&point.kind, CfgPointKind::Assign { target, .. } if reference_places_overlap(target, place))
            || matches!(&point.kind, CfgPointKind::ScopeExit if point.source_span.range == place.binding.block_span)
        {
            continue;
        }
        pending.extend(&point.successors);
    }
    false
}

fn live_callable_loan_conflicts(
    points: &[ReferenceFlowPoint],
    donor_uses: &BTreeMap<CfgPointId, ReferenceBindingId>,
    facts: &[ReferenceOrigin],
    environment_facts: &[CallableEnvironmentFact],
    state: &ReferenceEdgeState,
    point: CfgPointId,
    place: &ReferencePlaceId,
    access: ScalarReferenceMutability,
    authority: AccessAuthority,
) -> Vec<SourceSpan> {
    let environments = state
        .callable_bindings
        .iter()
        .filter(|(binding, _)| reference_read_reachable(points, donor_uses, point, binding))
        .flat_map(|(_, environments)| environments.iter())
        .chain(
            state
                .callable_projected
                .iter()
                .filter(|known| callable_field_read_reachable(points, donor_uses, point, &known.place))
                .flat_map(|known| known.environments.iter()),
        );
    let mut conflicts = Vec::new();
    for environment in environments {
        for loan_id in &environment_facts[environment.index].loans {
            let origin = &facts[loan_id.origin.0];
            let loan = match origin {
                ReferenceOrigin::Fresh { loan, .. }
                | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => continue,
            };
            if reborrow::authorizes(facts, authority, loan) {
                continue;
            }
            if loan_borrows_place(loan, place)
                && (access == ScalarReferenceMutability::Mutable
                    || loan.mode == ScalarReferenceMutability::Mutable)
            {
                let source = SourceSpan::new(loan.creation_source.clone(), loan.creation_span);
                if !conflicts.contains(&source) {
                    conflicts.push(source);
                }
            }
        }
    }
    conflicts
}

fn live_reference_borrow_conflicts(
    points: &[ReferenceFlowPoint],
    donor_uses: &BTreeMap<CfgPointId, ReferenceBindingId>,
    facts: &[ReferenceOrigin],
    state: &ReferenceEdgeState,
    point: CfgPointId,
    loan: &ReferenceLoanId,
) -> Vec<SourceSpan> {
    let place = available_place(&loan.origin_place);
    let origins = state
        .bindings
        .iter()
        .filter(|(binding, _)| reference_read_reachable(points, donor_uses, point, binding))
        .flat_map(|(_, origins)| origins.iter())
        .chain(
            state
                .projected
                .iter()
                .filter(|known| callable_field_read_reachable(points, donor_uses, point, &known.place))
                .flat_map(|known| known.origins.iter()),
        );
    let mut conflicts = Vec::new();
    for origin in origins {
        let active = match &facts[origin.0] {
            ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                loan
            }
            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => continue,
        };
        if loan
            .parent
            .as_deref()
            .is_some_and(|parent| reborrow::loan_authorizes(parent, active))
        {
            continue;
        }
        if (loan.mode == ScalarReferenceMutability::Mutable
            || active.mode == ScalarReferenceMutability::Mutable)
            && indexed_loan_borrows_place(state, active, &place)
        {
            let source = SourceSpan::new(active.creation_source.clone(), active.creation_span);
            if !conflicts.contains(&source) {
                conflicts.push(source);
            }
        }
    }
    conflicts
}

fn live_captured_backing_origins(
    points: &[ReferenceFlowPoint],
    donor_uses: &BTreeMap<CfgPointId, ReferenceBindingId>,
    environments: &[CallableEnvironmentFact],
    state: &ReferenceEdgeState,
    point: CfgPointId,
    binding: &ReferenceBindingId,
) -> Vec<SourceSpan> {
    let holders = state
        .callable_bindings
        .iter()
        .filter(|(holder, _)| reference_read_reachable(points, donor_uses, point, holder))
        .flat_map(|(_, environments)| environments.iter())
        .chain(
            state
                .callable_projected
                .iter()
                .filter(|known| callable_field_read_reachable(points, donor_uses, point, &known.place))
                .flat_map(|known| known.environments.iter()),
        );
    let mut origins = Vec::new();
    for environment in holders {
        for input in &environments[environment.index].inputs {
            if &input.binding == binding {
                for fact in &input.backings {
                    if matches!(fact.owner.kind, ReferenceBindingKind::Captured { .. }) {
                        if let Some(backings) = state.backing_bindings.get(binding) {
                            for active in
                                backings.iter().filter(|active| active.owner == fact.owner)
                            {
                                if !origins.contains(&active.origin) {
                                    origins.push(active.origin.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    origins
}

fn conflicting_live_loan(
    points: &[ReferenceFlowPoint],
    donor_uses: &BTreeMap<CfgPointId, ReferenceBindingId>,
    facts: &[ReferenceOrigin],
    environments: &[CallableEnvironmentFact],
    state: &ReferenceEdgeState,
    point: CfgPointId,
    place: &ReferencePlaceId,
    access: ScalarReferenceMutability,
    authority: AccessAuthority,
) -> Vec<SourceSpan> {
    let place = available_place(place);
    let mut conflicts = state
        .bindings
        .iter()
        .filter(|(binding, _)| reference_read_reachable(points, donor_uses, point, binding))
        .flat_map(|(_, origins)| origins.iter())
        .chain(
            state
                .projected
                .iter()
                .filter(|known| callable_field_read_reachable(points, donor_uses, point, &known.place))
                .flat_map(|known| known.origins.iter()),
        )
        .filter_map(|origin| match &facts[origin.0] {
            ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. }
                if !reborrow::authorizes(facts, authority, loan)
                    && (access == ScalarReferenceMutability::Mutable
                        || loan.mode == ScalarReferenceMutability::Mutable)
                    && indexed_loan_borrows_place(state, loan, &place) =>
            {
                Some(SourceSpan::new(
                    loan.creation_source.clone(),
                    loan.creation_span,
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    conflicts.extend(live_callable_loan_conflicts(
        points,
        donor_uses,
        facts,
        environments,
        state,
        point,
        &place,
        access,
        authority,
    ));
    conflicts.dedup();
    conflicts
}

fn storage_contains(storage: &ReferencePlaceId, holder: &ReferencePlaceId) -> bool {
    holder.projections.len() >= storage.projections.len()
        && reference_places_same(
            storage,
            &ReferencePlaceId {
                projections: holder.projections[..storage.projections.len()].to_vec(),
                ..holder.clone()
            },
        )
}

fn stored_reference_origins(
    state: &ReferenceEdgeState,
    destination: &ReferencePlaceId,
) -> BTreeSet<ReferenceOriginId> {
    let storage = available_place(destination);
    let mut origins = BTreeSet::new();
    if storage.projections.is_empty() {
        if let Some(stored) = state.bindings.get(&storage.binding) {
            origins.extend(stored);
        }
    }
    for holder in &state.projected {
        if storage_contains(&storage, &holder.place) {
            origins.extend(&holder.origins);
        }
    }
    origins
}

fn kill_stored_reference_origins(state: &mut ReferenceEdgeState, destination: &ReferencePlaceId) {
    let storage = available_place(destination);
    if storage.projections.is_empty() {
        state.bindings.remove(&storage.binding);
        state.uninitialized.remove(&storage.binding);
        state.released.remove(&storage.binding);
        state.invalidated.remove(&storage.binding);
    }
    state
        .projected
        .retain(|holder| !storage_contains(&storage, &holder.place));
}

fn apply_checked_return(
    state: &mut ReferenceEdgeState,
    facts: &[ReferenceOrigin],
    allocations: &BTreeSet<ReferenceBindingId>,
    point: &ReferenceFlowPoint,
    checked_value: &Option<BTreeSet<ReferenceOriginId>>,
    returned_backings: &[RuntimeBackingFact],
    errors: &mut Vec<ReferenceAnalysisError>,
) {
    if matches!(point.kind, CfgPointKind::ReturnOutput { checked: true, .. })
        && checked_value.as_ref().is_none_or(BTreeSet::is_empty)
    {
        errors.push(ReferenceAnalysisError::MissingContext {
            source_span: point.source_span.clone(),
            reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
        });
    }
    if let (CfgPointKind::ReturnOutput { checked: true, .. }, Some(origins)) =
        (&point.kind, checked_value)
    {
        for origin in origins {
            let loan = match &facts[origin.0] {
                ReferenceOrigin::Fresh { loan, .. }
                | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => continue,
            };
            if unavailable_place(state, &loan.origin_place)
                || state.released.contains(&loan.origin_place.binding)
                || state.invalidated.contains(&loan.origin_place.binding)
                || allocations.contains(&loan.origin_place.binding)
                    && state
                        .backing_bindings
                        .get(&loan.origin_place.binding)
                        .is_some_and(|backings| {
                            backings.iter().any(|backing| {
                                backing.kind == RuntimeBackingKind::OwnedAllocation
                                    && !backing.callable_family.is_empty()
                                    && !returned_backings.iter().any(|returned| {
                                        returned.value == backing.value
                                            && returned.owner == backing.owner
                                    })
                            })
                        })
            {
                errors.push(ReferenceAnalysisError::ConflictingPlace {
                    source_span: point.source_span.clone(),
                    origins: vec![SourceSpan::new(
                        loan.creation_source.clone(),
                        loan.creation_span,
                    )],
                });
            }
        }
    }
    if let (CfgPointKind::ReturnOutput { output, .. }, Some(origins)) = (&point.kind, checked_value)
    {
        if !origins.is_empty() {
            state.return_outputs.insert(*output, origins.clone());
        }
    }
}

fn checked_return_input(
    state: &ReferenceEdgeState,
    value_point: CfgPointId,
) -> Option<BTreeSet<ReferenceOriginId>> {
    state.point_values[value_point.0].clone()
}

fn read_checked_binding(
    state: &ReferenceEdgeState,
    checked_bindings: &BTreeMap<ReferenceBindingId, ReferencePlaceId>,
    point: &ReferenceFlowPoint,
    binding: &ReferenceBindingId,
    errors: &mut Vec<ReferenceAnalysisError>,
) -> Option<BTreeSet<ReferenceOriginId>> {
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
    if !checked_bindings.contains_key(binding) {
        None
    } else if state
        .moved
        .iter()
        .any(|(place, _)| &place.binding == binding)
        || state.released.contains(binding)
        || state.invalidated.contains(binding)
    {
        Some(BTreeSet::new())
    } else {
        match state
            .bindings
            .get(binding)
            .filter(|origins| !origins.is_empty() && !state.uninitialized.contains(binding))
        {
            Some(origins) => Some(origins.clone()),
            None => {
                errors.push(ReferenceAnalysisError::MissingOrigin {
                    source_span: point.source_span.clone(),
                    binding_id: binding.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                });
                Some(BTreeSet::new())
            }
        }
    }
}

fn publish_checked_value(
    point: &mut ReferenceFlowPoint,
    checked_value: &Option<BTreeSet<ReferenceOriginId>>,
) {
    if matches!(
        point.kind,
        CfgPointKind::Read { .. }
            | CfgPointKind::Borrow { .. }
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
}

fn save_checked_value(state: &mut ReferenceEdgeState, point: &ReferenceFlowPoint) {
    state.point_values[point.id.0] = point.possible_origins.clone();
}

fn checked_install_input(
    state: &ReferenceEdgeState,
    install: &Option<ReferenceCheckedInstall>,
) -> BTreeSet<ReferenceOriginId> {
    match install {
        Some(ReferenceCheckedInstall {
            input: ReferenceCheckedInstallInput::Evaluated { value_point },
            ..
        }) => match &state.point_values[value_point.0] {
            Some(origins) => origins.clone(),
            None => BTreeSet::new(),
        },
        Some(ReferenceCheckedInstall {
            input: ReferenceCheckedInstallInput::Parameter { origin, .. },
            ..
        }) => BTreeSet::from([*origin]),
        None => BTreeSet::new(),
    }
}

fn apply_checked_install(
    state: &mut ReferenceEdgeState,
    owner: &ProducerCfgOwnerId,
    checked_bindings: &BTreeMap<ReferenceBindingId, ReferencePlaceId>,
    point: &ReferenceFlowPoint,
    frozen_context: Option<&[FrozenAssignmentDestination]>,
    checked_install: &Option<ReferenceCheckedInstall>,
    checked_input: &BTreeSet<ReferenceOriginId>,
    evaluations: &mut Vec<ReferenceCheckedInstallEvaluation>,
    errors: &mut Vec<ReferenceAnalysisError>,
) -> Option<BTreeSet<ReferenceOriginId>> {
    match &point.kind {
        CfgPointKind::Bind { binding } if checked_bindings.contains_key(binding) => {
            let install = checked_install
                .as_ref()
                .expect("registered checked Bind input");
            let target = &checked_bindings[binding];
            install_checked_reference(
                state,
                owner,
                point.id,
                &install.input,
                target,
                checked_input,
                true,
                evaluations,
            );
            if checked_input.is_empty() {
                errors.push(ReferenceAnalysisError::MissingOrigin {
                    source_span: point.source_span.clone(),
                    binding_id: binding.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                });
            }
            Some(checked_input.clone())
        }
        CfgPointKind::Assign { target, .. } => {
            let projected_target = match frozen_context {
                Some(rows) => assignment_frozen_flow::direct_place(rows),
                None => state
                    .selected_element
                    .as_ref()
                    .map(|(_, place)| place)
                    .unwrap_or(target),
            };
            let storage = available_place(projected_target);
            let binding = &storage.binding;
            if let Some(install) = checked_install {
                install_checked_reference(
                    state,
                    owner,
                    point.id,
                    &install.input,
                    &storage,
                    checked_input,
                    false,
                    evaluations,
                );
                if checked_input.is_empty() {
                    errors.push(ReferenceAnalysisError::MissingOrigin {
                        source_span: point.source_span.clone(),
                        binding_id: binding.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                    });
                }
                Some(checked_input.clone())
            } else {
                kill_stored_reference_origins(state, &storage);
                None
            }
        }
        CfgPointKind::AssignThrough { candidates, .. } => {
            if let Some(install) = checked_install {
                for candidate in candidates {
                    install_checked_reference(
                        state,
                        owner,
                        point.id,
                        &install.input,
                        &candidate.place,
                        checked_input,
                        false,
                        evaluations,
                    );
                }
                if checked_input.is_empty() {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                    });
                }
                Some(checked_input.clone())
            } else {
                for candidate in candidates {
                    kill_stored_reference_origins(state, &candidate.place);
                }
                None
            }
        }
        _ => unreachable!("checked install consumer"),
    }
}

fn install_checked_reference(
    state: &mut ReferenceEdgeState,
    owner: &ProducerCfgOwnerId,
    point: CfgPointId,
    input: &ReferenceCheckedInstallInput,
    target: &ReferencePlaceId,
    origins: &BTreeSet<ReferenceOriginId>,
    initializer: bool,
    evaluations: &mut Vec<ReferenceCheckedInstallEvaluation>,
) {
    let target = available_place(target);
    let previous_origins = stored_reference_origins(state, &target);
    let disposition = if origins.is_empty() {
        if initializer {
            state.bindings.remove(&target.binding);
            state.uninitialized.insert(target.binding.clone());
            ReferenceCheckedInstallDisposition::Uninitialized
        } else {
            ReferenceCheckedInstallDisposition::Retained
        }
    } else {
        kill_stored_reference_origins(state, &target);
        state
            .moved
            .retain(|(moved, _)| !storage_contains(&target, moved));
        if target.projections.is_empty() {
            state
                .bindings
                .insert(target.binding.clone(), origins.clone());
        } else {
            write_projected_reference(state, &target, origins);
        }
        ReferenceCheckedInstallDisposition::Written
    };
    let evaluation = ReferenceCheckedInstallEvaluation {
        owner: owner.clone(),
        point,
        input: input.clone(),
        target,
        previous_origins,
        accepted_origins: origins.clone(),
        disposition,
    };
    if !evaluations.contains(&evaluation) {
        evaluations.push(evaluation);
    }
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

fn runtime_call_signatures(
    cfg: &ScalarReferenceCfg,
    state: &ReferenceEdgeState,
    target: &ResolvedCallTarget,
    type_arguments: &[ScalarTypeArgument],
) -> Vec<RuntimeCallSignature> {
    match target {
        ResolvedCallTarget::ModuleCallable {
            source,
            declaration_span,
            concrete,
        } => cfg
            .call_signatures
            .iter()
            .filter(|signature| {
                signature.source == *source
                    && signature.declaration_span == *declaration_span
                    && match (&signature.selection, concrete) {
                        (RuntimeCallSelection::Declaration, ConcreteCallSelection::Declaration) => {
                            true
                        }
                        (
                            RuntimeCallSelection::Overload(index),
                            ConcreteCallSelection::Overload(selection),
                        ) => *index == selection.arm_index,
                        _ => false,
                    }
            })
            .map(|signature| {
                let mut signature = signature.clone();
                let substitutions = match concrete {
                    ConcreteCallSelection::Overload(selection) => selection.substitutions.clone(),
                    ConcreteCallSelection::Declaration => signature
                        .generic_parameters
                        .iter()
                        .zip(type_arguments)
                        .map(|(name, ty)| (name.clone(), ty.ty.clone()))
                        .collect(),
                };
                signature.parameters = signature
                    .parameters
                    .iter()
                    .map(|ty| substitute_type(ty, &substitutions))
                    .collect();
                signature
            })
            .collect(),
        ResolvedCallTarget::LocalCallable(binding) => state
            .callable_bindings
            .get(binding)
            .into_iter()
            .flat_map(|environments| environments.iter())
            .flat_map(|environment| {
                cfg.initial_points
                    .iter()
                    .filter_map(move |point| match &point.kind {
                        CfgPointKind::Capture {
                            environment: captured,
                            signature:
                                ScalarType::Callable {
                                    parameters,
                                    outputs,
                                },
                            parameter_spans,
                            body,
                            ..
                        } if captured == environment => Some(RuntimeCallSignature {
                            source: environment.literal.source.clone(),
                            declaration_span: environment.literal.range,
                            selection: RuntimeCallSelection::Declaration,
                            parameters: parameters.clone(),
                            outputs: outputs
                                .outputs
                                .iter()
                                .map(|output| output.ty.clone())
                                .collect(),
                            generic_parameters: Vec::new(),
                            bindings: parameter_spans
                                .iter()
                                .map(|span| ReferenceBindingId {
                                    source: environment.literal.source.clone(),
                                    function_span: environment.literal.range,
                                    declaration_span: *span,
                                    block_span: body.span,
                                    kind: ReferenceBindingKind::Declared,
                                })
                                .collect(),
                        }),
                        _ => None,
                    })
            })
            .collect(),
        ResolvedCallTarget::CallableField {
            receiver_place,
            field,
        } => {
            let mut place = receiver_place.clone();
            place.projections.push(ReferencePlaceProjection::Field(
                ScalarFieldReference::Resolved(field.clone()),
            ));
            state
                .callable_projected
                .iter()
                .filter(|candidate| reference_places_same(&candidate.place, &place))
                .flat_map(|candidate| &candidate.environments)
                .flat_map(|environment| {
                    cfg.call_signatures.iter().filter(move |signature| {
                        signature.source == environment.literal.source
                            && signature.declaration_span == environment.literal.range
                            && signature.selection == RuntimeCallSelection::Declaration
                    })
                })
                .cloned()
                .collect()
        }
        _ => Vec::new(),
    }
}

use super::reference_producers::{
    build_capture_input_path_fact, build_producer_return_case_fact, record_holder_creation_roots,
    return_case_selection_refuted, return_environment_from_inputs, validated_entry_producer,
    PendingCapturePath, PendingReturnCase, PendingReturnGap,
};
pub(super) use super::reference_producers::{
    validate_capture_input_path_fact, validate_producer_return_case_fact,
};

fn rebase_mapped_origins(
    origins: &BTreeSet<ReferenceOriginId>,
    origin_ids: &[ReferenceOriginId],
) -> Option<BTreeSet<ReferenceOriginId>> {
    origins
        .iter()
        .map(|origin| origin_ids.get(origin.0).copied())
        .collect()
}

fn rebase_mapped_loans(
    loans: &BTreeSet<CallableLoanId>,
    origin_ids: &[ReferenceOriginId],
) -> Option<BTreeSet<CallableLoanId>> {
    loans
        .iter()
        .map(|loan| {
            origin_ids
                .get(loan.origin.0)
                .map(|origin| CallableLoanId { origin: *origin })
        })
        .collect()
}

fn rebase_mapped_environments(
    environments: &BTreeSet<CallableEnvironmentId>,
    environment_ids: &[CallableEnvironmentId],
) -> Option<BTreeSet<CallableEnvironmentId>> {
    environments
        .iter()
        .map(|environment| environment_ids.get(environment.index).cloned())
        .collect()
}

fn rebase_mapped_cell_domain(
    cell: &CallableCellDomainFact,
    environment_ids: &[CallableEnvironmentId],
    origin_ids: &[ReferenceOriginId],
) -> Option<CallableCellDomainFact> {
    let mut alternatives = Vec::new();
    for case in &cell.alternatives {
        let rebased = rebase_capture_cell_case(case, environment_ids, origin_ids)?;
        if !alternatives.contains(&rebased) {
            alternatives.push(rebased);
        }
    }
    Some(CallableCellDomainFact { alternatives })
}

fn rebase_mapped_backing(
    backing: &RuntimeBackingFact,
    environment_ids: &[CallableEnvironmentId],
    origin_ids: &[ReferenceOriginId],
) -> Option<RuntimeBackingFact> {
    let mut rebased = backing.clone();
    for family in &mut rebased.callable_family {
        for element in &mut family.elements {
            element.environment = environment_ids.get(element.environment.index)?.clone();
            element.loans = rebase_mapped_loans(&element.loans, origin_ids)?;
        }
        for partition in &mut family.partitions {
            for representative in &mut partition.representatives {
                representative.cell =
                    rebase_mapped_cell_domain(&representative.cell, environment_ids, origin_ids)?;
            }
            partition.remainder =
                rebase_mapped_cell_domain(&partition.remainder, environment_ids, origin_ids)?;
        }
    }
    Some(rebased)
}

fn rebase_mapped_projected_backing(
    projected: &RuntimeProjectedBackingFact,
    environment_ids: &[CallableEnvironmentId],
    origin_ids: &[ReferenceOriginId],
) -> Option<RuntimeProjectedBackingFact> {
    Some(RuntimeProjectedBackingFact {
        place: projected.place.clone(),
        backing: rebase_mapped_backing(&projected.backing, environment_ids, origin_ids)?,
    })
}

fn rebase_mapped_capture_inputs(
    inputs: &[CallableCaptureInput],
    environment_ids: &[CallableEnvironmentId],
    origin_ids: &[ReferenceOriginId],
) -> Option<Vec<CallableCaptureInput>> {
    inputs
        .iter()
        .map(|input| {
            Some(CallableCaptureInput {
                binding: input.binding.clone(),
                origins: rebase_mapped_origins(&input.origins, origin_ids)?,
                environments: rebase_mapped_environments(&input.environments, environment_ids)?,
                projected: input
                    .projected
                    .iter()
                    .map(|projection| {
                        Some(CallableProjectedEnvironment {
                            place: projection.place.clone(),
                            environments: rebase_mapped_environments(
                                &projection.environments,
                                environment_ids,
                            )?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
                backings: input
                    .backings
                    .iter()
                    .map(|backing| rebase_mapped_backing(backing, environment_ids, origin_ids))
                    .collect::<Option<Vec<_>>>()?,
                projected_backings: input
                    .projected_backings
                    .iter()
                    .map(|projected| {
                        rebase_mapped_projected_backing(projected, environment_ids, origin_ids)
                    })
                    .collect::<Option<Vec<_>>>()?,
            })
        })
        .collect()
}

fn rebase_capture_cell_case(
    case: &CallableCellCaseFact,
    environment_ids: &[CallableEnvironmentId],
    origin_ids: &[ReferenceOriginId],
) -> Option<CallableCellCaseFact> {
    match case {
        CallableCellCaseFact::Uninitialized => Some(CallableCellCaseFact::Uninitialized),
        CallableCellCaseFact::Initialized {
            environment_origin,
            instance,
            holder_aliases,
            backing,
            owner,
            capture_paths,
            loans,
            moved,
            origin,
        } => Some(CallableCellCaseFact::Initialized {
            environment_origin: environment_ids.get(environment_origin.index)?.clone(),
            instance: instance.clone(),
            holder_aliases: holder_aliases.clone(),
            backing: backing.clone(),
            owner: owner.clone(),
            capture_paths: capture_paths
                .iter()
                .map(|path| rebase_mapped_capture_inputs(path, environment_ids, origin_ids))
                .collect::<Option<Vec<_>>>()?,
            loans: rebase_mapped_loans(loans, origin_ids)?,
            moved: *moved,
            origin: origin.clone(),
        }),
    }
}

fn rebase_holder_action(
    action: &CallableHolderAction,
    environment_ids: &[CallableEnvironmentId],
    origin_ids: &[ReferenceOriginId],
) -> Option<CallableHolderAction> {
    match action {
        CallableHolderAction::Retain {
            environment,
            from,
            to,
            span,
        } => Some(CallableHolderAction::Retain {
            environment: environment_ids.get(environment.index)?.clone(),
            from: from.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::Move {
            environment,
            from,
            to,
            span,
        } => Some(CallableHolderAction::Move {
            environment: environment_ids.get(environment.index)?.clone(),
            from: from.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::RetainSelected {
            environment,
            from,
            to,
            span,
        } => Some(CallableHolderAction::RetainSelected {
            environment: environment_ids.get(environment.index)?.clone(),
            from: from.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::MoveSelected {
            environment,
            from,
            to,
            span,
        } => Some(CallableHolderAction::MoveSelected {
            environment: environment_ids.get(environment.index)?.clone(),
            from: from.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::RetainCheckedSelected {
            holder,
            old,
            guard,
            to,
            span,
        } => Some(CallableHolderAction::RetainCheckedSelected {
            holder: holder.clone(),
            old: rebase_capture_cell_case(old, environment_ids, origin_ids)?,
            guard: guard.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
        _ => None,
    }
}

fn rebase_holder_producer_ref(
    producer: &HolderProducerRef,
    environment_ids: &[CallableEnvironmentId],
    origin_ids: &[ReferenceOriginId],
) -> Option<HolderProducerRef> {
    match producer {
        HolderProducerRef::CaptureCreation { point, environment } => {
            Some(HolderProducerRef::CaptureCreation {
                point: *point,
                environment: environment_ids.get(environment.index)?.clone(),
            })
        }
        HolderProducerRef::CallCreation {
            point,
            environment,
            selection,
        } => Some(HolderProducerRef::CallCreation {
            point: *point,
            environment: environment_ids.get(environment.index)?.clone(),
            selection: ProducerCallSelectionFact {
                call: selection.call,
                target: selection.target.clone(),
                output: selection.output,
                source: selection.source.clone(),
                destination: environment_ids.get(selection.destination.index)?.clone(),
                projection: selection.projection.clone(),
            },
        }),
        HolderProducerRef::InstalledRead {
            point,
            environment,
            action,
        } => Some(HolderProducerRef::InstalledRead {
            point: *point,
            environment: environment_ids.get(environment.index)?.clone(),
            action: rebase_holder_action(action, environment_ids, origin_ids)?,
        }),
        HolderProducerRef::GlobalCallsiteInput {
            call,
            holder,
            environment,
        } => Some(HolderProducerRef::GlobalCallsiteInput {
            call: call.clone(),
            holder: holder.clone(),
            environment: environment_ids.get(environment.index)?.clone(),
        }),
        HolderProducerRef::Forward {
            point,
            environment,
            action,
            previous,
        } => Some(HolderProducerRef::Forward {
            point: *point,
            environment: environment_ids.get(environment.index)?.clone(),
            action: rebase_holder_action(action, environment_ids, origin_ids)?,
            previous: Box::new(rebase_holder_producer_ref(
                previous,
                environment_ids,
                origin_ids,
            )?),
        }),
    }
}

fn capture_value_cell_droppable(ty: &ScalarType) -> bool {
    // A captured value cell holds a copy of the binding. Dropping it at the
    // binding's scope exit is sound exactly when the value cannot own
    // callable holders that would need their own connected release.
    // Aggregate, pointer, reference and unresolved types keep the previous
    // behavior (no value-cell action).
    matches!(
        ty,
        ScalarType::Unit
            | ScalarType::Bool
            | ScalarType::I8
            | ScalarType::I16
            | ScalarType::I32
            | ScalarType::I64
            | ScalarType::I128
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
            | ScalarType::U128
            | ScalarType::F32
            | ScalarType::F64
            | ScalarType::Char
            | ScalarType::ArtifactId
            | ScalarType::Callable { .. }
    )
}

fn receipt_counted_pair_needed(
    cfg: &ScalarReferenceCfg,
    environment: &CallableEnvironmentId,
) -> bool {
    // A received escaping environment enters the counted domain through a
    // connected field Retain plus its balancing Drop. Both actions are
    // balance-neutral on their edge (Retain then Drop) and only confer
    // counted status so the connected terminal Drop emits exactly once per
    // generation. The pair fires only when no existing action already
    // counts the environment and no cell/array path owns its release;
    // otherwise the existing coverage (or its absence by design) stands.
    let mut has_drop = false;
    let mut counted = false;
    let mut drop_bindings: Vec<ReferenceBindingId> = Vec::new();
    for action in cfg
        .holder_edges
        .iter()
        .flat_map(|edge| &edge.actions)
        .chain(cfg.holder_exits.iter().flat_map(|exit| &exit.actions))
    {
        match action {
            CallableHolderAction::Drop {
                environment: actual,
                holder,
                ..
            } if actual == environment => {
                has_drop = true;
                match holder {
                    CallableHolderPlace::Binding(binding) => {
                        if !drop_bindings.contains(binding) {
                            drop_bindings.push(binding.clone());
                        }
                    }
                    CallableHolderPlace::Projection(place) => {
                        if !drop_bindings.contains(&place.binding) {
                            drop_bindings.push(place.binding.clone());
                        }
                        if place.projections.iter().any(|projection| {
                            matches!(projection, ReferencePlaceProjection::Index(_))
                        }) {
                            counted = true;
                        }
                    }
                    _ => {}
                }
            }
            CallableHolderAction::MoveSelected {
                environment: actual,
                ..
            }
            | CallableHolderAction::RetainSelected {
                environment: actual,
                ..
            } => {
                if actual == environment {
                    counted = true;
                }
            }
            CallableHolderAction::RetainCheckedSelected {
                old:
                    CallableCellCaseFact::Initialized {
                        environment_origin, ..
                    },
                ..
            } => {
                if environment_origin == environment {
                    counted = true;
                }
            }
            CallableHolderAction::Retain {
                environment: actual,
                from: CallableHolderPlace::Projection(place),
                ..
            } => {
                if actual == environment
                    && place.source == environment.literal.source
                    && place
                        .projections
                        .iter()
                        .any(|projection| matches!(projection, ReferencePlaceProjection::Field(_)))
                {
                    counted = true;
                }
            }
            _ => {}
        }
    }
    if !has_drop || counted {
        return false;
    }
    for action in cfg
        .holder_edges
        .iter()
        .flat_map(|edge| &edge.actions)
        .chain(cfg.holder_exits.iter().flat_map(|exit| &exit.actions))
    {
        match action {
            CallableHolderAction::DropCaptureCellOwner { binding, .. }
            | CallableHolderAction::DropCapturedValueCellOwner { binding, .. }
            | CallableHolderAction::ReplaceCaptureCellValue { binding, .. } => {
                if drop_bindings.contains(binding) {
                    return false;
                }
            }
            CallableHolderAction::DropArrayElements { backing, .. } => {
                if drop_bindings
                    .iter()
                    .any(|binding| binding == &backing.owner)
                {
                    return false;
                }
            }
            CallableHolderAction::RetainArrayElements {
                source, backing, ..
            } => {
                if drop_bindings
                    .iter()
                    .any(|binding| binding == &source.owner || binding == &backing.owner)
                {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

#[cfg(test)]
pub(super) fn retransfer_evaluated_borrow_test(
    cfg: &mut ScalarReferenceCfg,
    capture_input: &source_boundary::SourceCapturePreflightInput,
) -> Vec<ReferenceAnalysisError> {
    if let Err(error) = source_boundary::validate_capture_preflight(cfg, capture_input) { return vec![error]; }
    cfg.points = cfg.initial_points.clone();
    transfer_reference_facts(
        cfg,
        &BTreeMap::new(),
        &[],
        &[],
        &[],
        capture_input,
        #[cfg(test)]
        ReferenceTransferTestRole::Function,
        #[cfg(test)]
        &mut ReferenceValidationTestTrace::new(),
    )
}

fn transfer_reference_facts(
    cfg: &mut ScalarReferenceCfg,
    deferred_policies: &BTreeMap<usize, CopyPolicy>,
    contracts: &[ConcreteReferenceOutputContract],
    literal_contracts: &[LiteralOutputContract],
    call_exits: &[GlobalCallExitSnapshot],
    capture_input: &source_boundary::SourceCapturePreflightInput,
    #[cfg(test)] role: ReferenceTransferTestRole,
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    transfer_reference_facts_observed(
        cfg,
        deferred_policies,
        contracts,
        literal_contracts,
        call_exits,
        capture_input,
        #[cfg(test)]
        role,
        #[cfg(test)]
        ReferenceTransferTestStage::Revalidation,
        #[cfg(test)]
        trace,
    )
    .errors
}

fn transfer_reference_facts_observed(
    cfg: &mut ScalarReferenceCfg,
    deferred_policies: &BTreeMap<usize, CopyPolicy>,
    contracts: &[ConcreteReferenceOutputContract],
    literal_contracts: &[LiteralOutputContract],
    call_exits: &[GlobalCallExitSnapshot],
    capture_input: &source_boundary::SourceCapturePreflightInput,
    #[cfg(test)] role: ReferenceTransferTestRole,
    #[cfg(test)] stage: ReferenceTransferTestStage,
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> name_resolution::initial_transfer::InitialTransferReceipt {
    #[cfg(test)]
    let id = ReferenceTransferTestPassId(trace.passes.len());
    #[cfg(test)]
    let _snapshot23_worker = selected_source::snapshot23_observation::worker(
        &cfg.producer_owner,
        id.0,
        matches!(&role, ReferenceTransferTestRole::Function),
        usize::from(matches!(&stage, ReferenceTransferTestStage::Revalidation)),
        trace as *const ReferenceValidationTestTrace as usize,
    );
    #[cfg(test)]
    let cfg_at_entry = cfg.clone();
    #[cfg(test)]
    let context = ReferenceTransferTestContext {
        capture_input: capture_input.clone(),
        deferred_policies: deferred_policies.clone(),
        contracts: contracts.to_vec(),
        literal_contracts: literal_contracts.to_vec(),
        call_exits: call_exits.to_vec(),
    };
    #[cfg(test)]
    let mut observations = ReferenceTransferTestObservations {
        borrows: Vec::new(),
        points: Vec::new(),
    };
    let mut borrows = Vec::new();
    let errors = transfer_reference_facts_worker(
        cfg,
        deferred_policies,
        contracts,
        literal_contracts,
        call_exits,
        capture_input,
        &mut borrows,
        #[cfg(test)]
        &mut observations,
    );
    #[cfg(test)]
    trace.passes.push(ReferenceTransferTestPass {
        id,
        owner: cfg.producer_owner.clone(),
        role,
        stage,
        context,
        cfg_at_entry,
        cfg_at_exit: cfg.clone(),
        observations,
        errors: errors.clone(),
    });
    name_resolution::initial_transfer::InitialTransferReceipt { errors, borrows }
}

fn validate_and_apply_reference_borrow_cases(
    owner: &ProducerCfgOwnerId,
    facts: &[ReferenceOrigin],
    environments: &[CallableEnvironmentFact],
    original_points: &[ReferenceFlowPoint],
    donor_uses: &BTreeMap<CfgPointId, ReferenceBindingId>,
    state: &mut ReferenceEdgeState,
    point: &ReferenceFlowPoint,
    request: &ReferenceBorrowRequest,
    input: ReferenceBorrowInput,
    mut cases: Vec<ReferenceBorrowCase>,
    errors: &mut Vec<ReferenceAnalysisError>,
) -> ReferenceBorrowEvaluation {
    let id = point.id;
    let function_span = original_points[0].source_span.range;
    let mut borrow_output = BTreeSet::new();
    for case in &mut cases {
        let rejected_input = match &case {
            ReferenceBorrowCase::Accepted {
                access: ReferenceBorrowAccess::DirectStorage,
                ..
            } => ReferenceBorrowRejectedInput::DirectStorage,
            ReferenceBorrowCase::Accepted {
                access: ReferenceBorrowAccess::Through { parent_origin, .. },
                ..
            }
            | ReferenceBorrowCase::Invalid { parent_origin, .. } => {
                ReferenceBorrowRejectedInput::Through {
                    parent_origin: *parent_origin,
                }
            }
            ReferenceBorrowCase::Rejected { .. } => {
                unreachable!("unvalidated realization")
            }
        };
        if !evaluated_borrow::validates(facts, point, function_span, request, &input, case) {
            let reason = ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin;
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: point.source_span.clone(),
                reason,
            });
            *case = ReferenceBorrowCase::Rejected {
                input: rejected_input,
                diagnostic: ReferenceBorrowDiagnostic::Missing {
                    source_span: point.source_span.clone(),
                    reason,
                },
            };
            continue;
        }
        match case {
            ReferenceBorrowCase::Accepted {
                place,
                loan,
                origin,
                access,
            } => {
                let authority = match access {
                    ReferenceBorrowAccess::DirectStorage => AccessAuthority::DirectStorage,
                    ReferenceBorrowAccess::Through { parent_origin, .. } => {
                        AccessAuthority::Through(*parent_origin)
                    }
                };
                let mut conflicts = unavailable_origins(state, place);
                conflicts.extend(live_callable_loan_conflicts(
                    original_points,
                    donor_uses,
                    facts,
                    environments,
                    state,
                    id,
                    &available_place(place),
                    loan.mode,
                    authority,
                ));
                conflicts.extend(live_reference_borrow_conflicts(
                    original_points,
                    donor_uses,
                    facts,
                    state,
                    id,
                    loan,
                ));
                conflicts.dedup();
                if !conflicts.is_empty() {
                    errors.push(ReferenceAnalysisError::ConflictingPlace {
                        source_span: point.source_span.clone(),
                        origins: conflicts.clone(),
                    });
                    *case = ReferenceBorrowCase::Rejected {
                        input: rejected_input,
                        diagnostic: ReferenceBorrowDiagnostic::Conflict {
                            source_span: point.source_span.clone(),
                            origins: conflicts,
                        },
                    };
                } else if state.released.contains(&place.binding)
                    || state.invalidated.contains(&place.binding)
                {
                    let reason = ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin;
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason,
                    });
                    *case = ReferenceBorrowCase::Rejected {
                        input: rejected_input,
                        diagnostic: ReferenceBorrowDiagnostic::Missing {
                            source_span: point.source_span.clone(),
                            reason,
                        },
                    };
                } else {
                    pin_indexed_reference_loan(
                        state,
                        original_points,
                        id,
                        loan,
                        evaluated_borrow::indices(request),
                    );
                    borrow_output.insert(*origin);
                }
            }
            ReferenceBorrowCase::Invalid { origin, .. } => {
                borrow_output.insert(*origin);
            }
            ReferenceBorrowCase::Rejected { .. } => unreachable!("realization status"),
        }
    }
    ReferenceBorrowEvaluation {
        owner: owner.clone(),
        point: id,
        input,
        cases,
        output: borrow_output,
    }
}

fn transfer_reference_facts_worker(
    cfg: &mut ScalarReferenceCfg,
    deferred_policies: &BTreeMap<usize, CopyPolicy>,
    contracts: &[ConcreteReferenceOutputContract],
    literal_contracts: &[LiteralOutputContract],
    call_exits: &[GlobalCallExitSnapshot],
    capture_input: &source_boundary::SourceCapturePreflightInput,
    origin_receipts: &mut Vec<name_resolution::initial_transfer::InitialBorrowOriginReceipt>,
    #[cfg(test)] observations: &mut ReferenceTransferTestObservations,
) -> Vec<ReferenceAnalysisError> {
    if let Err(error) = source_boundary::validate_signatures(cfg) {
        return vec![error];
    }
    if let Err(error) = owner_lifetime::validate(cfg) {
        return vec![error];
    }
    let parameter_points = match parameter_input::introductions(cfg) {
        Ok(points) => points,
        Err(error) => return vec![error],
    };
    for input in &cfg.evaluated_values.call_argument_inputs {
        if let Err(error) = source_boundary::validate_call_arguments(cfg, input) {
            return vec![error];
        }
    }
    if let Err(error) = output_sequence::validate_binding_inputs(cfg) {
        return vec![error];
    }
    if let Err(error) = binding_holder::validate(cfg) { return vec![error]; }
    if let Err(error) = conditional_output_production::validate(cfg) {
        return vec![error];
    }
    if let Err(error) = source_boundary::validate_capture_preflight(cfg, capture_input) { return vec![error]; }
    if let Err(error) = common_loan::register(cfg) {
        return vec![error];
    }
    let frozen_protocol = match assignment_lifecycle::FrozenAssignmentProtocol::new(cfg) {
        Ok(protocol) => protocol,
        Err(error) => return vec![error],
    };
    cfg.borrow_evaluations.clear();
    cfg.allocation_address_evaluations.clear();
    cfg.rebind_evaluations.clear();
    cfg.checked_install_evaluations.clear();
    for point in &cfg.initial_points {
        if let CfgPointKind::Capture { environment, .. } = &point.kind {
            let fact = &mut cfg.environments[environment.index];
            fact.origins.clear();
            fact.owners.clear();
            fact.loans.clear();
            fact.backings.clear();
            fact.inputs.clear();
            fact.capture_paths.clear();
        }
    }
    let original_points = cfg.points.clone();
    let original_read_cfg = cfg.clone();
    let donor_uses = binding_holder::uses(cfg);
    let mut last_use = (0..cfg.points.len()).collect::<Vec<_>>();
    for point in &original_points {
        let dependencies = match &point.kind {
            CfgPointKind::AssignmentDestination {
                storage: EvaluatedDestinationStorage::Through { pointer, .. },
                ..
            } => vec![pointer.point],
            CfgPointKind::AssignmentSnapshot {
                destination_point, ..
            } => vec![*destination_point],
            CfgPointKind::AssignmentResult { snapshot_point, .. }
            | CfgPointKind::AssignmentDispose { snapshot_point } => vec![*snapshot_point],
            CfgPointKind::Invalidate { address_point, .. }
            | CfgPointKind::Release {
                target: ReleaseTarget::Checked { address_point, .. },
            } => vec![*address_point],
            CfgPointKind::MoveThrough { pointer_point, .. }
            | CfgPointKind::DeferredMoveThrough { pointer_point, .. } => vec![*pointer_point],
            CfgPointKind::Borrow { request } => {
                let mut inputs = evaluated_borrow::indices(request)
                    .iter()
                    .map(|input| input.value_point)
                    .collect::<Vec<_>>();
                if let ReferenceBorrowRequest::Through { pointer_point, .. } = request {
                    inputs.push(*pointer_point);
                }
                inputs
            }
            CfgPointKind::Assign { rhs_point, .. }
            | CfgPointKind::BindArrayElement { rhs_point, .. } => vec![*rhs_point],
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
            last_use[dependency.0] = last_use[dependency.0].max(point.id.0);
        }
    }
    for install in &cfg.checked_installs {
        if let ReferenceCheckedInstallInput::Evaluated { value_point } = install.input {
            last_use[value_point.0] = last_use[value_point.0].max(install.point.0);
        }
    }
    for input in &cfg.evaluated_values.binding_inputs {
        last_use[input.source.point.0] = last_use[input.source.point.0].max(input.consumer.0);
        if let Some(first) = binding_holder::first(cfg, input) {
            last_use[first.consumer.0] = last_use[first.consumer.0].max(input.consumer.0);
        }
    }
    for receiving in &cfg.binding_points {
        if receiving.role == ScalarBindingPointRole::CopyCallableFields {
            if let Some(input) = binding_holder::field_input(cfg, receiving.point).expect("validated field input") {
                last_use[input.source.point.0] = last_use[input.source.point.0].max(receiving.point.0);
            }
            let receipt = evaluated_aggregate_receiving::plan(cfg, receiving.point).expect("validated actual aggregate receipt");
            if let Some(first) = binding_holder::field_first(cfg, receiving.point, Some(&receipt)).expect("validated first field receiver") {
                last_use[first.0] = last_use[first.0].max(receiving.point.0);
            }
        }
    }
    for production in cfg.evaluated_values.productions.iter().rev() {
        let inputs = match &production.source {
            OutputProductionSource::Conditional { inputs, .. } => inputs.iter()
                .flat_map(|input| &input.outputs).collect::<Vec<_>>(),
            OutputProductionSource::Block { inputs, .. } => inputs.iter().collect(),
            _ => Vec::new(),
        };
        for source in inputs {
            for output in &production.outputs {
                last_use[source.point.0] = last_use[source.point.0].max(last_use[output.point.0]);
            }
        }
    }
    let mut observed = vec![None::<ReferenceFlowPoint>; cfg.points.len()];
    cfg.holder_edges.clear();
    cfg.holder_exits.clear();
    cfg.nested_call_installs.clear();
    cfg.capture_input_path_facts
        .retain(|fact| fact.owner != cfg.producer_owner);
    cfg.producer_return_cases
        .retain(|fact| fact.table.owner != cfg.producer_owner);
    let mut incoming = vec![Vec::<ReferenceEdgeState>::new(); cfg.points.len()];
    let initial_state = ReferenceEdgeState {
        assignment_destinations: BTreeMap::new(),
        assignment_saved: BTreeMap::new(),
        backing_bindings: cfg
            .inputs
            .iter()
            .filter(|input| !input.backings.is_empty())
            .map(|input| (input.binding.clone(), input.backings.clone()))
            .collect(),
        bindings: cfg
            .inputs
            .iter()
            .filter(|input| !input.origins.is_empty())
            .map(|input| (input.binding.clone(), input.origins.clone()))
            .collect(),
        callable_bindings: cfg
            .inputs
            .iter()
            .filter(|input| !input.environments.is_empty())
            .map(|input| (input.binding.clone(), input.environments.clone()))
            .collect(),
        callable_projected: cfg
            .inputs
            .iter()
            .flat_map(|input| input.projected.iter().cloned())
            .collect(),
        backing_projected: cfg
            .inputs
            .iter()
            .flat_map(|input| input.projected_backings.iter().cloned())
            .collect(),
        point_values: vec![None; cfg.points.len()],
        allocation_address_values: vec![None; cfg.points.len()],
        callable_point_values: vec![BTreeSet::new(); cfg.points.len()],
        callable_point_projected: vec![Vec::new(); cfg.points.len()],
        holder_point_values: vec![Vec::new(); cfg.points.len()],
        holder_point_sources: vec![Vec::new(); cfg.points.len()],
        ..ReferenceEdgeState::new(FrameVariableRef {
            owner: cfg.producer_owner.clone(),
            variable: SupportedInstanceVariable(0),
        })
    };
    let capture_paths = if cfg.capture_input_paths.is_empty() {
        vec![cfg.inputs.clone()]
    } else {
        cfg.capture_input_paths.clone()
    };
    let mut entry_errors = Vec::new();
    let mut entry_bases: Vec<ReferenceEdgeState> = Vec::new();
    let entry_span = match &cfg.producer_owner {
        ProducerCfgOwnerId::Literal { literal, .. } => Some(literal.clone()),
        _ => None,
    };
    for capture_path in capture_paths {
        let mut captured_state = initial_state.clone();
        captured_state.backing_bindings = capture_path
            .iter()
            .filter(|input| !input.backings.is_empty())
            .map(|input| (input.binding.clone(), input.backings.clone()))
            .collect();
        captured_state.bindings = capture_path
            .iter()
            .filter(|input| !input.origins.is_empty())
            .map(|input| (input.binding.clone(), input.origins.clone()))
            .collect();
        captured_state.callable_bindings = capture_path
            .iter()
            .filter(|input| !input.environments.is_empty())
            .map(|input| (input.binding.clone(), input.environments.clone()))
            .collect();
        captured_state.callable_projected = capture_path
            .iter()
            .flat_map(|input| input.projected.iter().cloned())
            .collect();
        captured_state.backing_projected = capture_path
            .iter()
            .flat_map(|input| input.projected_backings.iter().cloned())
            .collect();
        for input in &capture_path {
            for environment in &input.environments {
                let holder = CallableHolderPlace::Binding(input.binding.clone());
                if let Some(span) = &entry_span {
                    let Some(producer) = validated_entry_producer(
                        &cfg.capture_input_path_facts,
                        &input.binding,
                        &holder,
                        environment,
                    ) else {
                        entry_errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: span.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                        });
                        continue;
                    };
                    captured_state.holder_provenance.insert(
                        environment.clone(),
                        holder.clone(),
                        producer,
                    );
                }
                captured_state
                    .holder_relations
                    .push(CallableHolderRelation {
                        environment: environment.clone(),
                        holders: vec![CallableHolderPlace::Binding(input.binding.clone())],
                        capture_paths: cfg.environments[environment.index].capture_paths.clone(),
                    });
            }
            for projected in &input.projected {
                for environment in &projected.environments {
                    let holder = CallableHolderPlace::Projection(projected.place.clone());
                    if let Some(span) = &entry_span {
                        let Some(producer) = validated_entry_producer(
                            &cfg.capture_input_path_facts,
                            &input.binding,
                            &holder,
                            environment,
                        ) else {
                            entry_errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                            });
                            continue;
                        };
                        captured_state.holder_provenance.insert(
                            environment.clone(),
                            holder.clone(),
                            producer,
                        );
                    }
                    captured_state
                        .holder_relations
                        .push(CallableHolderRelation {
                            environment: environment.clone(),
                            holders: vec![CallableHolderPlace::Projection(projected.place.clone())],
                            capture_paths: cfg.environments[environment.index]
                                .capture_paths
                                .clone(),
                        });
                }
            }
        }
        normalize_holder_relations(&mut captured_state.holder_relations);
        if cfg.backing_input_paths.is_empty() && cfg.projected_backing_input_paths.is_empty() {
            for parameter in &cfg.backing_parameters {
                captured_state.backing_bindings.insert(
                    parameter.clone(),
                    vec![RuntimeBackingFact {
                        value: RuntimeValueId {
                            source: parameter.source.clone(),
                            function_span: parameter.function_span,
                            creation_span: parameter.declaration_span,
                            position: RuntimeValuePosition::Parameter {
                                declaration_span: parameter.declaration_span,
                            },
                        },
                        owner: parameter.clone(),
                        kind: RuntimeBackingKind::OwnedAllocation,
                        origin: SourceSpan::new(
                            parameter.source.clone(),
                            parameter.declaration_span,
                        ),
                        length: RuntimeArrayLengthFact::Parameter {
                            binding: parameter.clone(),
                        },
                        callable_family: cfg
                            .callable_array_parameters
                            .iter()
                            .filter(|known| known.binding == *parameter)
                            .map(|known| CallableArrayFamilyFact {
                                element: known.element.clone(),
                                elements: Vec::new(),
                                index_relations: Vec::new(),
                                unselected: CallableArrayUnselectedFact {
                                    selected: Vec::new(),
                                },
                                partitions: vec![CallableIndexedPartitionFact {
                                    representatives: Vec::new(),
                                    relations: Vec::new(),
                                    remainder: CallableCellDomainFact {
                                        alternatives: vec![CallableCellCaseFact::Uninitialized],
                                    },
                                }],
                            })
                            .collect(),
                    }],
                );
            }
            if !entry_bases.contains(&captured_state) {
                entry_bases.push(captured_state.clone());
            }
        } else if cfg.projected_backing_input_paths.is_empty() {
            for path in &cfg.backing_input_paths {
                let mut state = captured_state.clone();
                for fact in path {
                    state
                        .backing_bindings
                        .entry(fact.owner.clone())
                        .or_default()
                        .push(fact.clone());
                }
                if !entry_bases.contains(&state) {
                    entry_bases.push(state);
                }
            }
        }
        if !cfg.projected_backing_input_paths.is_empty() {
            for path in &cfg.projected_backing_input_paths {
                let mut projected_state = captured_state.clone();
                for input in path {
                    if input.projections.is_empty() {
                        projected_state
                            .backing_bindings
                            .entry(input.backing.owner.clone())
                            .or_default()
                            .push(input.backing.clone());
                        continue;
                    }
                    let parameter = ReferenceBindingId {
                        kind: ReferenceBindingKind::Declared,
                        ..input.backing.owner.clone()
                    };
                    let projected = RuntimeProjectedBackingFact {
                        place: ReferencePlaceId {
                            source: parameter.source.clone(),
                            function_span: parameter.function_span,
                            declaration_span: parameter.declaration_span,
                            block_span: parameter.block_span,
                            binding: parameter.clone(),
                            projections: input.projections.clone(),
                            place: ScalarPlace::Name {
                                name: String::new(),
                                span: parameter.declaration_span,
                            },
                        },
                        backing: input.backing.clone(),
                    };
                    if !projected_state.backing_projected.contains(&projected) {
                        projected_state.backing_projected.push(projected);
                    }
                }
                if !entry_bases.contains(&projected_state) {
                    entry_bases.push(projected_state);
                }
            }
        }
    }
    if cfg.global_callable_input_paths.is_empty() {
        for base in entry_bases {
            if !incoming[0].contains(&base) {
                incoming[0].push(base);
            }
        }
    } else {
        for base in entry_bases {
            for input in cfg.global_callable_input_paths.clone() {
                let mut seeded = base.clone();
                match seed_global_callsite_entry(cfg, &mut seeded, &input) {
                    Ok(()) => {
                        if !incoming[0].contains(&seeded) {
                            incoming[0].push(seeded);
                        }
                    }
                    Err(error) => entry_errors.push(error),
                }
            }
        }
    }
    let mut visited = vec![Vec::<ReferenceEdgeState>::new(); cfg.points.len()];
    let selected_root = match selected_source::root(cfg) {
        Ok(root) => root,
        Err(error) => return vec![error],
    };
    let mut incoming = incoming
        .into_iter()
        .map(|states| {
            states
                .into_iter()
                .map(|state| ReferenceWorkItem {
                    state,
                    selected: selected_root.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut selected_reach = vec![Vec::<ReferenceWorkItem>::new(); cfg.points.len()];
    let mut queued = vec![false; cfg.points.len()];
    queued[0] = true;
    let mut ready = std::collections::VecDeque::from([CfgPointId(0)]);
    let mut errors = entry_errors;
    let mut pending_captures = Vec::new();
    let mut pending_returns: Vec<PendingReturnCase> = Vec::new();
    let mut pending_return_gaps: Vec<PendingReturnGap> = Vec::new();
    let mut pending_refuted: Vec<(CfgPointId, usize, CallableEnvironmentId)> = Vec::new();
    while let Some(id) = ready.pop_front() {
        queued[id.0] = false;
        let multiple_paths = incoming[id.0].len() > 1;
        for work in std::mem::take(&mut incoming[id.0]) {
            reference_state::admit(&mut visited[id.0], &work.state);
            if !admit_reference_work_item(&mut selected_reach[id.0], work.clone()) {
                continue;
            }
            let ReferenceWorkItem {
                mut state,
                selected: selected_input,
            } = work;
            #[cfg(test)]
            observations.points.push(ReferenceTransferPointSnapshot {
                point: id,
                phase: ReferenceTransferPhase::BeforeEvaluation,
                state: state.clone(),
            });
            if matches!(
                &original_points[id.0].kind,
                CfgPointKind::Evaluate(ScalarExpression::RawAddress { .. })
            ) {
                match evaluated_raw_address::production(cfg, id) {
                    Ok(production) => {
                        if matches!(production.source, RawAddressInputSource::CompilerStorage) {
                            let input = match evaluated_raw_address::source_input(cfg, id) {
                                Ok(input) => input,
                                Err(error) => {
                                    errors.push(error);
                                    continue;
                                }
                            };
                            if unavailable_place(&state, &input.place) {
                                errors.push(ReferenceAnalysisError::ConflictingPlace {
                                    source_span: original_points[id.0].source_span.clone(),
                                    origins: unavailable_origins(&state, &input.place),
                                });
                                continue;
                            }
                        }
                    }
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                }
            }
            match &original_points[id.0].kind {
                CfgPointKind::Bind { binding } if !parameter_points.contains(&id) => {
                    if let Some(CfgPointKind::Evaluate(ScalarExpression::Integer {
                        value, ..
                    })) =
                        id.0.checked_sub(1)
                            .map(|previous| &original_points[previous].kind)
                    {
                        state.index_values.insert(binding.clone(), value.clone());
                    } else {
                        state.index_values.remove(binding);
                    }
                }
                CfgPointKind::Assign {
                    target, rhs_point, ..
                } if target.projections.is_empty() => {
                    if let CfgPointKind::Evaluate(ScalarExpression::Integer { value, .. }) =
                        &original_points[rhs_point.0].kind
                    {
                        state
                            .index_values
                            .insert(target.binding.clone(), value.clone());
                    } else {
                        state.index_values.remove(&target.binding);
                    }
                }
                CfgPointKind::ScopeExit => {
                    state.index_values.retain(|binding, _| {
                        binding.block_span != original_points[id.0].source_span.range
                    });
                }
                _ => {}
            }
            let holder_before = state.holder_relations.clone();
            let mut holder_actions = Vec::new();
            let mut release_cleanup = Vec::new();
            let mut move_alternatives = Vec::new();
            let mut element_alternatives = Vec::new();
            let mut call_exit_extra_states = Vec::new();
            cfg.points[id.0] = original_points[id.0].clone();
            let binding_input = output_sequence::binding_input(cfg, id).cloned();
            let control_value = cfg.evaluated_values.productions.iter().any(|production| {
                production.producer == cfg.producer_owner
                    && matches!(production.source, OutputProductionSource::Conditional { .. } | OutputProductionSource::Block { .. })
                    && production.outputs.iter().any(|value| value.point == id)
            });
            if let CfgPointKind::Call {
                target, output: 0, ..
            } = &original_points[id.0].kind
            {
                cfg.points[id.0].global_callable_call_paths =
                    vec![global_callable_call_paths(cfg, &state, target)];
            }
            if let CfgPointKind::SelectArrayElement { target, .. } = &original_points[id.0].kind {
                let candidate_target = available_place(target);
                element_alternatives = state
                    .holder_relations
                    .iter()
                    .flat_map(|relation| &relation.holders)
                    .filter_map(|holder| match holder {
                        CallableHolderPlace::Projection(place)
                            if place.projections.len() == candidate_target.projections.len()
                                && reference_places_overlap(place, &candidate_target)
                                && place
                                    .projections
                                    .iter()
                                    .zip(&candidate_target.projections)
                                    .all(|(candidate, selection)| {
                                        match (candidate, selection) {
                                            (
                                                ReferencePlaceProjection::Index(
                                                    ScalarExpression::Integer { .. },
                                                ),
                                                ReferencePlaceProjection::Index(_),
                                            ) => true,
                                            (
                                                ReferencePlaceProjection::Index(_),
                                                ReferencePlaceProjection::Index(_),
                                            ) => false,
                                            _ => true,
                                        }
                                    }) =>
                        {
                            Some(place.clone())
                        }
                        _ => None,
                    })
                    .collect();
                element_alternatives.dedup();
                if candidate_target.projections.iter().any(|projection| matches!(projection, ReferencePlaceProjection::Index(expression) if !matches!(expression, ScalarExpression::Integer { .. })))
                    && state.backing_bindings.get(&candidate_target.binding).is_some_and(|facts| facts.iter().any(|fact| fact.kind == RuntimeBackingKind::OwnedAllocation && !fact.callable_family.is_empty()))
                {
                    element_alternatives.clear();
                    element_alternatives.push(candidate_target.clone());
                }
                if element_alternatives.is_empty() {
                    if state
                        .backing_bindings
                        .get(&candidate_target.binding)
                        .is_some_and(|facts| {
                            facts.iter().any(|fact| {
                                fact.kind == RuntimeBackingKind::OwnedAllocation
                                    && !fact.callable_family.is_empty()
                            })
                        })
                    {
                        element_alternatives.push(candidate_target);
                    } else {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: cfg.points[id.0].source_span.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        });
                    }
                }
                if let CfgPointKind::SelectArrayElement { index_point, .. } =
                    &original_points[id.0].kind
                {
                    if target
                        .projections
                        .iter()
                        .any(|projection| matches!(projection, ReferencePlaceProjection::Index(_)))
                    {
                        let occurrence = target
                            .projections
                            .iter()
                            .rev()
                            .find_map(|projection| match projection {
                                ReferencePlaceProjection::Index(expression) => {
                                    Some(SourceSpan::new(
                                        target.source.clone(),
                                        expression_span(expression),
                                    ))
                                }
                                _ => None,
                            })
                            .expect("selected index expression");
                        let evaluated = CallableIndexKeyFact::Evaluated {
                            point: *index_point,
                            occurrence,
                            function_span: target.function_span,
                        };
                        let live = match &original_points[index_point.0].kind {
                            CfgPointKind::Read { binding } => {
                                let ScalarPlace::Index { index, .. } = &target.place else {
                                    unreachable!()
                                };
                                let ScalarExpression::Name { name, .. } = index.as_ref() else {
                                    unreachable!()
                                };
                                Some(CallableIndexKeyFact::LivePlace(ReferencePlaceId {
                                    source: binding.source.clone(),
                                    function_span: binding.function_span,
                                    declaration_span: binding.declaration_span,
                                    block_span: binding.block_span,
                                    binding: binding.clone(),
                                    projections: Vec::new(),
                                    place: ScalarPlace::Name {
                                        name: name.clone(),
                                        span: binding.declaration_span,
                                    },
                                }))
                            }
                            _ => None,
                        };
                        if let Some(backings) = state.backing_bindings.get_mut(&target.binding) {
                            for backing in backings {
                                if backing.kind != RuntimeBackingKind::OwnedAllocation {
                                    continue;
                                }
                                for family in &mut backing.callable_family {
                                    let constant = target
                                        .projections
                                        .iter()
                                        .rev()
                                        .find_map(|projection| match projection {
                                            ReferencePlaceProjection::Index(
                                                ScalarExpression::Integer { value, .. },
                                            ) => Some(value.clone()),
                                            _ => None,
                                        })
                                        .or_else(|| {
                                            live.as_ref().and_then(|key| match key {
                                                CallableIndexKeyFact::LivePlace(place) => {
                                                    state.index_values.get(&place.binding).cloned()
                                                }
                                                _ => unreachable!(),
                                            })
                                        });
                                    evaluate_indexed_family(
                                        family,
                                        evaluated.clone(),
                                        live.clone(),
                                        constant,
                                        *index_point,
                                    );
                                }
                            }
                        }
                    }
                }
            }
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
            let checked_install = cfg
                .checked_installs
                .iter()
                .find(|install| install.point == id)
                .cloned();
            let checked_input = checked_install_input(&state, &checked_install);
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
            let call_template = match &cfg.points[id.0].kind {
                CfgPointKind::Call {
                    expression: ScalarExpression::Call { type_arguments, .. },
                    output,
                    target:
                        ResolvedCallTarget::ModuleCallable {
                            source,
                            declaration_span,
                            concrete,
                        },
                    ..
                } => contracts
                    .iter()
                    .find(|contract| {
                        contract.source == *source
                            && contract.declaration_span == *declaration_span
                            && contract.selection == *concrete
                            && contract.type_arguments
                                == type_arguments
                                    .iter()
                                    .map(|argument| argument.ty.clone())
                                    .collect::<Vec<_>>()
                            && contract.output == *output
                    })
                    .map(|contract| contract.origins.clone()),
                CfgPointKind::Call {
                    output,
                    target:
                        ResolvedCallTarget::CallableField {
                            receiver_place,
                            field,
                        },
                    ..
                } if !literal_contracts.is_empty() => {
                    let mut place = receiver_place.clone();
                    place.projections.push(ReferencePlaceProjection::Field(
                        ScalarFieldReference::Resolved(field.clone()),
                    ));
                    let environments = state
                        .callable_projected
                        .iter()
                        .filter(|candidate| reference_places_same(&candidate.place, &place))
                        .flat_map(|candidate| candidate.environments.iter());
                    let mut origins = Vec::new();
                    for environment in environments {
                        for contract in literal_contracts.iter().filter(|contract| {
                            contract.literal == environment.literal && contract.output == *output
                        }) {
                            for origin in &contract.origins {
                                if !origins.contains(origin) {
                                    origins.push(origin.clone());
                                }
                            }
                        }
                    }
                    Some(origins)
                }
                _ => None,
            };
            let call_origins = if let (
                Some(template),
                CfgPointKind::Call {
                    argument_points,
                    output,
                    ..
                },
            ) = (call_template, &cfg.points[id.0].kind)
            {
                Some({
                    let mut results = BTreeSet::new();
                    for origin in &template {
                        let actual = match origin {
                            ReferenceOrigin::BorrowedFrom { parameter, .. } => {
                                if let Some(argument) = argument_points
                                    .get(*parameter)
                                    .and_then(|point| state.point_values[point.0].as_ref())
                                {
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
                                loan.creation_source = cfg.points[id.0].source_span.source.clone();
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
                        let origin_id = if let Some(index) =
                            cfg.facts.iter().position(|fact| fact == &actual)
                        {
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
            let call_kind = cfg.points[id.0].kind.clone();
            let mut call_import_path = Vec::new();
            let call_environments = if let CfgPointKind::Call {
                expression: ScalarExpression::Call { type_arguments, .. },
                output,
                target:
                    ResolvedCallTarget::ModuleCallable {
                        source,
                        declaration_span,
                        concrete,
                    },
                argument_points,
            } = &call_kind
            {
                let templates = contracts
                    .iter()
                    .filter(|contract| {
                        contract.source == *source
                            && contract.declaration_span == *declaration_span
                            && contract.selection == *concrete
                            && contract.output == *output
                            && contract.type_arguments
                                == type_arguments
                                    .iter()
                                    .map(|argument| argument.ty.clone())
                                    .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                let instance = CallableEnvironmentInstance::CallOutput {
                    call: cfg.points[id.0].source_span.clone(),
                    output: *output,
                    function_span: cfg.points[0].source_span.range,
                };
                let mut environments = BTreeSet::new();
                let mut projected = Vec::new();
                for template in templates {
                    for result in &template.environments {
                        if let Some(environment) = import_call_result(
                            cfg,
                            result,
                            &instance,
                            id,
                            &call_kind,
                            Vec::new(),
                            &mut call_import_path,
                            &mut errors,
                        ) {
                            environments.insert(environment);
                        }
                    }
                    for (place, result) in &template.projected {
                        let Some(environment) = import_call_result(
                            cfg,
                            result,
                            &instance,
                            id,
                            &call_kind,
                            place.projections.clone(),
                            &mut call_import_path,
                            &mut errors,
                        ) else {
                            continue;
                        };
                        let mut place = place.clone();
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
                        place.source = binding.source.clone();
                        place.function_span = binding.function_span;
                        place.declaration_span = binding.declaration_span;
                        place.block_span = binding.block_span;
                        place.binding = binding;
                        projected.push(CallableProjectedEnvironment {
                            place,
                            environments: BTreeSet::from([environment]),
                        });
                    }
                }
                for projection in &projected {
                    if let Some(known) = state
                        .callable_projected
                        .iter_mut()
                        .find(|known| reference_places_same(&known.place, &projection.place))
                    {
                        known
                            .environments
                            .extend(projection.environments.iter().cloned());
                    } else {
                        state.callable_projected.push(projection.clone());
                    }
                }
                let projected = merge_call_projected(projected);
                cfg.points[id.0].possible_projected_environments = projected.clone();
                let mut imported = environments.clone();
                for projection in &projected {
                    imported.extend(projection.environments.iter().cloned());
                }
                let module_parameters =
                    module_callable_parameter_bindings(cfg, source, *declaration_span, concrete);
                let call_span = cfg.points[id.0].source_span.clone();
                forward_imported_environment_backings(
                    cfg,
                    &state,
                    &call_span,
                    &[],
                    Some(&module_parameters),
                    &imported,
                );
                substitute_imported_param_loans(
                    cfg,
                    &state,
                    argument_points,
                    &module_parameters,
                    &instance,
                    &imported,
                );
                Some(environments)
            } else if let CfgPointKind::Call {
                target: ResolvedCallTarget::LocalCallable(binding),
                output,
                ..
            } = &call_kind
            {
                let contracts = state
                    .callable_bindings
                    .get(binding)
                    .into_iter()
                    .flatten()
                    .flat_map(|environment| {
                        literal_contracts.iter().filter_map(move |contract| {
                            (contract.literal == environment.literal && contract.output == *output)
                                .then_some((environment, contract))
                        })
                    })
                    .collect::<Vec<_>>();
                let instance = CallableEnvironmentInstance::CallOutput {
                    call: cfg.points[id.0].source_span.clone(),
                    output: *output,
                    function_span: cfg.points[0].source_span.range,
                };
                let call_span = cfg.points[id.0].source_span.clone();
                let function_span = cfg.points[0].source_span.range;
                let mut environments = BTreeSet::new();
                let mut projected = Vec::new();
                for (callee, contract) in contracts {
                    for result in &contract.environments {
                        if !callee_produces_return(cfg, callee, result) {
                            continue;
                        }
                        if let Some(environment) = import_call_result(
                            cfg,
                            result,
                            &instance,
                            id,
                            &call_kind,
                            Vec::new(),
                            &mut call_import_path,
                            &mut errors,
                        ) {
                            environments.insert(environment);
                        }
                    }
                    for (place, result) in &contract.projected {
                        if !callee_produces_return(cfg, callee, result) {
                            continue;
                        }
                        let Some(environment) = import_call_result(
                            cfg,
                            result,
                            &instance,
                            id,
                            &call_kind,
                            place.projections.clone(),
                            &mut call_import_path,
                            &mut errors,
                        ) else {
                            continue;
                        };
                        let mut place = place.clone();
                        let call_binding = ReferenceBindingId {
                            source: call_span.source.clone(),
                            function_span,
                            declaration_span: call_span.range,
                            block_span: function_span,
                            kind: ReferenceBindingKind::CallOutput {
                                point: id,
                                output: *output,
                            },
                        };
                        place.source = call_binding.source.clone();
                        place.function_span = call_binding.function_span;
                        place.declaration_span = call_binding.declaration_span;
                        place.block_span = call_binding.block_span;
                        place.binding = call_binding;
                        projected.push(CallableProjectedEnvironment {
                            place,
                            environments: BTreeSet::from([environment]),
                        });
                    }
                }
                for projection in &projected {
                    if let Some(known) = state
                        .callable_projected
                        .iter_mut()
                        .find(|known| reference_places_same(&known.place, &projection.place))
                    {
                        known
                            .environments
                            .extend(projection.environments.iter().cloned());
                    } else {
                        state.callable_projected.push(projection.clone());
                    }
                }
                let projected = merge_call_projected(projected);
                cfg.points[id.0].possible_projected_environments = projected.clone();
                let mut imported = environments.clone();
                for projection in &projected {
                    imported.extend(projection.environments.iter().cloned());
                }
                let call_span = cfg.points[id.0].source_span.clone();
                forward_imported_environment_backings(
                    cfg,
                    &state,
                    &call_span,
                    &[],
                    None,
                    &imported,
                );
                Some(environments)
            } else if let CfgPointKind::Call {
                output,
                target:
                    ResolvedCallTarget::CallableField {
                        receiver_place,
                        field,
                    },
                ..
            } = &cfg.points[id.0].kind
            {
                if literal_contracts.is_empty() {
                    None
                } else {
                    let output = *output;
                    let mut place = receiver_place.clone();
                    place.projections.push(ReferencePlaceProjection::Field(
                        ScalarFieldReference::Resolved(field.clone()),
                    ));
                    let contracts = state
                        .callable_projected
                        .iter()
                        .filter(|candidate| reference_places_same(&candidate.place, &place))
                        .flat_map(|candidate| candidate.environments.iter())
                        .flat_map(|environment| {
                            literal_contracts.iter().filter_map(move |contract| {
                                (contract.literal == environment.literal
                                    && contract.output == output)
                                    .then_some((environment, contract))
                            })
                        })
                        .collect::<Vec<_>>();
                    let mut environments = BTreeSet::new();
                    let mut projected = Vec::new();
                    let call_span = cfg.points[id.0].source_span.clone();
                    let function_span = cfg.points[0].source_span.range;
                    let instance = CallableEnvironmentInstance::CallOutput {
                        call: call_span.clone(),
                        output,
                        function_span,
                    };
                    for (callee, contract) in contracts {
                        for result in &contract.environments {
                            if !callee_produces_return(cfg, callee, result) {
                                continue;
                            }
                            if let Some(environment) = import_call_result(
                                cfg,
                                result,
                                &instance,
                                id,
                                &call_kind,
                                Vec::new(),
                                &mut call_import_path,
                                &mut errors,
                            ) {
                                environments.insert(environment);
                            }
                        }
                        for (place, result) in &contract.projected {
                            if !callee_produces_return(cfg, callee, result) {
                                continue;
                            }
                            let Some(environment) = import_call_result(
                                cfg,
                                result,
                                &instance,
                                id,
                                &call_kind,
                                place.projections.clone(),
                                &mut call_import_path,
                                &mut errors,
                            ) else {
                                continue;
                            };
                            let mut place = place.clone();
                            let call_binding = ReferenceBindingId {
                                source: call_span.source.clone(),
                                function_span,
                                declaration_span: call_span.range,
                                block_span: function_span,
                                kind: ReferenceBindingKind::CallOutput { point: id, output },
                            };
                            place.source = call_binding.source.clone();
                            place.function_span = call_binding.function_span;
                            place.declaration_span = call_binding.declaration_span;
                            place.block_span = call_binding.block_span;
                            place.binding = call_binding;
                            projected.push(CallableProjectedEnvironment {
                                place,
                                environments: BTreeSet::from([environment]),
                            });
                        }
                    }
                    for projection in &projected {
                        if let Some(known) = state
                            .callable_projected
                            .iter_mut()
                            .find(|known| reference_places_same(&known.place, &projection.place))
                        {
                            known
                                .environments
                                .extend(projection.environments.iter().cloned());
                        } else {
                            state.callable_projected.push(projection.clone());
                        }
                    }
                    let projected = merge_call_projected(projected);
                    cfg.points[id.0].possible_projected_environments = projected.clone();
                    let mut imported = environments.clone();
                    for projection in &projected {
                        imported.extend(projection.environments.iter().cloned());
                    }
                    let call_span = cfg.points[id.0].source_span.clone();
                    forward_imported_environment_backings(
                        cfg,
                        &state,
                        &call_span,
                        &[],
                        None,
                        &imported,
                    );
                    Some(environments)
                }
            } else {
                None
            };
            let runtime_signatures = match &cfg.points[id.0].kind {
                CfgPointKind::Call {
                    expression: ScalarExpression::Call { type_arguments, .. },
                    output: 0,
                    target,
                    ..
                } => runtime_call_signatures(cfg, &state, target, type_arguments),
                _ => Vec::new(),
            };
            let current_function_span = cfg.points[0].source_span.range;
            let callee_environments = match &cfg.points[id.0].kind {
                CfgPointKind::Call {
                    target: ResolvedCallTarget::LocalCallable(binding),
                    ..
                } => match state.callable_bindings.get(binding) {
                    Some(environments) => environments.clone(),
                    None => BTreeSet::new(),
                },
                CfgPointKind::Call {
                    target:
                        ResolvedCallTarget::CallableField {
                            receiver_place,
                            field,
                        },
                    ..
                } => {
                    let mut place = receiver_place.clone();
                    place.projections.push(ReferencePlaceProjection::Field(
                        ScalarFieldReference::Resolved(field.clone()),
                    ));
                    state
                        .callable_projected
                        .iter()
                        .filter(|candidate| reference_places_same(&candidate.place, &place))
                        .flat_map(|candidate| candidate.environments.iter().cloned())
                        .collect()
                }
                _ => BTreeSet::new(),
            };
            let returned_backings = if matches!(
                original_points[id.0].kind,
                CfgPointKind::ReturnOutput { checked: true, .. }
            ) {
                cfg.points
                    .iter()
                    .filter(|candidate| {
                        matches!(
                            candidate.kind,
                            CfgPointKind::ReturnOutput { checked: false, .. }
                        )
                    })
                    .flat_map(|candidate| candidate.possible_backings.iter().cloned())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let value_projected = match &original_points[id.0].kind {
                CfgPointKind::ReturnOutput { value_point, .. } => cfg.points[value_point.0]
                    .possible_projected_backings
                    .clone(),
                _ => Vec::new(),
            };
            let first_binding_input = cfg.evaluated_values.binding_inputs.iter().find(|input| input.consumer == id)
                .and_then(|input| binding_holder::first(cfg, input)).cloned();
            let aggregate_receipt = match &original_points[id.0].kind {
                CfgPointKind::CopyCallableFields { source: CallableFieldSource::Evaluated(_), .. } =>
                    Some(evaluated_aggregate_receiving::plan(cfg, id).expect("validated actual aggregate receipt")),
                _ => None,
            };
            let field_prefix = if matches!(original_points[id.0].kind, CfgPointKind::CopyCallableFields { .. }) {
                match binding_holder::field_prefix(cfg, id, aggregate_receipt.as_ref()) {
                    Ok(prefix) => prefix,
                    Err(error) => { errors.push(error); continue; }
                }
            } else { Vec::new() };
            let field_route = if matches!(original_points[id.0].kind, CfgPointKind::CopyCallableFields { .. }) {
                Some(binding_holder::field_route(cfg, id, aggregate_receipt.as_ref()).expect("validated actual field route"))
            } else { None };
            let field_receiving = if matches!(original_points[id.0].kind, CfgPointKind::CopyCallableFields { .. }) {
                binding_holder::field_input(cfg, id).expect("validated actual field input").cloned()
            } else { None };
            let field_donor = if matches!(original_points[id.0].kind, CfgPointKind::CopyCallableFields { source: CallableFieldSource::Evaluated(_), .. }) {
                match &field_receiving {
                    Some(input) => {
                        let donor = match &cfg.points[input.source.point.0].kind {
                            CfgPointKind::Read { binding } => binding.clone(),
                            CfgPointKind::ReadPlace { candidates, .. } if candidates.len() == 1
                                && !matches!(candidates[0].binding.kind, ReferenceBindingKind::CallOutput { .. }) => candidates[0].binding.clone(),
                            _ => {
                                if !cfg.binding_points.iter().any(|binding| binding.point == input.consumer && binding.role == ScalarBindingPointRole::Bind) {
                                    errors.push(ReferenceAnalysisError::MissingContext {
                                        source_span: cfg.points[id.0].source_span.clone(),
                                        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                                    });
                                    continue;
                                }
                                let consumer = match binding_holder::first(cfg, input) {
                                    Some(first_input) => first_input.consumer,
                                    None => input.consumer,
                                };
                                let CfgPointKind::Bind { binding } = &cfg.points[consumer.0].kind else {
                                    errors.push(ReferenceAnalysisError::MissingContext {
                                        source_span: cfg.points[id.0].source_span.clone(),
                                        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                                    });
                                    continue;
                                };
                                binding.clone()
                            }
                        };
                        Some(donor)
                    }
                    None => {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: cfg.points[id.0].source_span.clone(),
                            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                        });
                        continue;
                    }
                }
            } else { None };
            let aggregate_read = matches!(original_points[id.0].kind, CfgPointKind::ReadPlace { .. })
                && cfg.evaluated_values.values.iter().filter(|value| value.id.point == id).any(|value|
                    cfg.evaluated_values.shapes.iter().any(|shape| shape.id == value.shape
                        && matches!(shape.ty, ScalarType::Struct(_) | ScalarType::Array { .. })));
            let point = &mut cfg.points[id.0];
            point.possible_callee_environments = callee_environments;
            if matches!(point.kind, CfgPointKind::Call { .. }) && !call_import_path.is_empty() {
                point.call_import_paths.push(call_import_path);
            }
            if matches!(point.kind, CfgPointKind::Call { .. }) {
                for environment in &point.possible_callee_environments {
                    for input in &cfg.environments[environment.index].inputs {
                        if let Some(backings) = state.backing_bindings.get(&input.binding) {
                            for backing in backings {
                                if matches!(
                                    backing.owner.kind,
                                    ReferenceBindingKind::Captured { .. }
                                ) && !point.possible_backings.contains(backing)
                                {
                                    point.possible_backings.push(backing.clone());
                                }
                            }
                        }
                        for projected in state
                            .backing_projected
                            .iter()
                            .filter(|known| known.place.binding == input.binding)
                        {
                            if matches!(
                                projected.backing.owner.kind,
                                ReferenceBindingKind::Captured { .. }
                            ) && !point.possible_backings.contains(&projected.backing)
                            {
                                point.possible_backings.push(projected.backing.clone());
                            }
                        }
                    }
                }
            }
            if let Some(origins) = call_origins {
                point.possible_origins = Some(origins);
            }
            if let Some(environments) = call_environments {
                point.possible_environments = environments;
            }
            if let CfgPointKind::Call {
                expression: ScalarExpression::Call { arguments, .. },
                argument_points,
                output: 0,
                ..
            } = &point.kind
            {
                let mut moved_sources = BTreeSet::new();
                let mut moved_aggregates = BTreeSet::new();
                for signature in &runtime_signatures {
                    let mut path = Vec::new();
                    let input_expressions = output_sequence::call_argument_expressions(
                        &cfg.evaluated_values,
                        arguments,
                    );
                    for (position, ((argument, parameter), binding)) in input_expressions
                        .iter()
                        .zip(&signature.parameters)
                        .zip(&signature.bindings)
                        .enumerate()
                    {
                        if matches!(parameter, ScalarType::Struct(_)) {
                            if let CfgPointKind::Read { binding } =
                                &original_points[argument_points[position].0].kind
                            {
                                if state
                                    .moved
                                    .iter()
                                    .any(|(place, _)| place.binding == *binding)
                                {
                                    moved_aggregates.insert(binding.clone());
                                }
                            }
                            let projected = match &original_points[argument_points[position].0].kind {
                                CfgPointKind::Read { binding } => state.backing_projected.iter()
                                    .filter(|known| known.place.binding == *binding)
                                    .map(|known| (known.clone(), known.place.projections.clone()))
                                    .collect::<Vec<_>>(),
                                CfgPointKind::ReadPlace { candidates, .. } => state.backing_projected.iter()
                                    .filter_map(|known| candidates.iter().find_map(|candidate| {
                                        (known.place.binding == candidate.binding
                                            && known.place.projections.starts_with(&candidate.projections))
                                            .then(|| (known.clone(), known.place.projections[candidate.projections.len()..].to_vec()))
                                    })).collect::<Vec<_>>(),
                                CfgPointKind::Call { .. } => state.backing_projected.iter()
                                    .filter(|known| matches!(known.place.binding.kind,
                                        ReferenceBindingKind::CallOutput { point, .. } if point == argument_points[position]))
                                    .map(|known| (known.clone(), known.place.projections.clone()))
                                    .collect::<Vec<_>>(),
                                _ => Vec::new(),
                            };
                            for (known, projections) in projected {
                                let field = match projections.last() {
                                    Some(ReferencePlaceProjection::Field(
                                        ScalarFieldReference::Resolved(field),
                                    )) => field.index,
                                    _ => unreachable!("typed aggregate runtime-array projection"),
                                };
                                let owner = ReferenceBindingId {
                                    kind: ReferenceBindingKind::Projected { field },
                                    ..binding.clone()
                                };
                                let mut backing = known.backing;
                                let previous_owner = backing.owner.clone();
                                transfer_array_family(&mut backing, &previous_owner, &owner, false);
                                backing.owner = owner;
                                path.push(RuntimeCallArgumentFact {
                                    position,
                                    projections,
                                    backing,
                                });
                            }
                            continue;
                        }
                        if !matches!(parameter, ScalarType::RuntimeArray { .. }) {
                            continue;
                        }
                        let (facts, source, span) =
                            match (argument, &original_points[argument_points[position].0].kind) {
                                (
                                    ScalarExpression::Name { span, .. },
                                    CfgPointKind::Read { binding },
                                ) => (
                                    state
                                        .backing_bindings
                                        .get(binding)
                                        .into_iter()
                                        .flatten()
                                        .cloned()
                                        .collect::<Vec<_>>(),
                                    Some(binding),
                                    *span,
                                ),
                                (ScalarExpression::Call { .. }, CfgPointKind::Call { .. }) => (
                                    state
                                        .backing_point_values
                                        .get(&argument_points[position])
                                        .into_iter()
                                        .flatten()
                                        .cloned()
                                        .collect::<Vec<_>>(),
                                    None,
                                    expression_span(argument),
                                ),
                                _ => continue,
                            };
                        for fact in facts {
                            let mut transferred = fact.clone();
                            if fact.kind == RuntimeBackingKind::FixedValue
                                && source.is_some_and(|source| {
                                    !state.moved.iter().any(|(place, origin)| {
                                        place.binding == *source && origin.range == span
                                    })
                                })
                            {
                                transferred.value = RuntimeValueId {
                                    source: point.source_span.source.clone(),
                                    function_span: fact.owner.function_span,
                                    creation_span: span,
                                    position: RuntimeValuePosition::Argument {
                                        call: point.source_span.range,
                                        index: position,
                                    },
                                };
                                transferred.origin =
                                    SourceSpan::new(point.source_span.source.clone(), span);
                                if let RuntimeArrayLengthFact::Fixed { value, .. } = &fact.length {
                                    transferred.length = RuntimeArrayLengthFact::Fixed {
                                        value: *value,
                                        origin: transferred.origin.clone(),
                                    };
                                }
                            } else {
                                moved_sources.insert(fact.owner.clone());
                            }
                            transfer_array_family(
                                &mut transferred,
                                &fact.owner,
                                binding,
                                fact.kind == RuntimeBackingKind::FixedValue,
                            );
                            transferred.owner = binding.clone();
                            transferred.kind = RuntimeBackingKind::OwnedAllocation;
                            path.push(RuntimeCallArgumentFact {
                                position,
                                projections: Vec::new(),
                                backing: transferred,
                            });
                        }
                    }
                    if !path.is_empty() {
                        for input in &path {
                            if !point.possible_backings.contains(&input.backing) {
                                point.possible_backings.push(input.backing.clone());
                            }
                        }
                        point.backing_argument_paths.push(path);
                    }
                }
                for source in moved_sources {
                    state.backing_bindings.remove(&source);
                }
                state
                    .backing_projected
                    .retain(|known| !moved_aggregates.contains(&known.place.binding));
                if !point.backing_argument_paths.is_empty() {
                    state
                        .backing_call_paths
                        .retain(|(span, _)| span != &point.source_span);
                    state.backing_call_paths.push((
                        point.source_span.clone(),
                        point.backing_argument_paths.clone(),
                    ));
                }
            }
            if let CfgPointKind::Call {
                target:
                    ResolvedCallTarget::ModuleCallable {
                        source,
                        declaration_span,
                        concrete,
                    },
                output,
                ..
            } = &point.kind
            {
                let selection = match concrete {
                    ConcreteCallSelection::Declaration => RuntimeCallSelection::Declaration,
                    ConcreteCallSelection::Overload(arm) => {
                        RuntimeCallSelection::Overload(arm.arm_index)
                    }
                };
                let paths = state
                    .backing_call_paths
                    .iter()
                    .find(|(span, _)| span == &point.source_span)
                    .map(|(_, paths)| paths.clone());
                let input_paths = if let Some(paths) = paths {
                    if paths.is_empty() {
                        vec![Vec::new()]
                    } else {
                        paths
                    }
                } else {
                    vec![Vec::new()]
                };
                for contract in cfg.backing_output_contracts.iter().filter(|contract| {
                    contract.source == *source
                        && contract.declaration_span == *declaration_span
                        && contract.selection == selection
                        && contract.output == *output
                }) {
                    for path in &input_paths {
                        for candidate in &contract.sources {
                            let fact = match candidate {
                                RuntimeOutputBackingSource::Parameter(position) => path
                                    .iter()
                                    .find(|input| {
                                        input.position == *position
                                            && input.projections == contract.projections
                                    })
                                    .map(|input| input.backing.clone()),
                                RuntimeOutputBackingSource::Created {
                                    origin,
                                    action,
                                    lineage,
                                    length,
                                    callable_family,
                                    owner: _,
                                } => Some(RuntimeBackingFact {
                                    value: RuntimeValueId {
                                        source: point.source_span.source.clone(),
                                        function_span: current_function_span,
                                        creation_span: point.source_span.range,
                                        position: RuntimeValuePosition::CallOutput {
                                            call: point.source_span.range,
                                            output: *output,
                                            target_source: source.clone(),
                                            target_declaration: *declaration_span,
                                            origin: origin.clone(),
                                            action: action.clone(),
                                            lineage: Box::new(lineage.clone()),
                                        },
                                    },
                                    owner: ReferenceBindingId {
                                        source: point.source_span.source.clone(),
                                        function_span: current_function_span,
                                        declaration_span: point.source_span.range,
                                        block_span: current_function_span,
                                        kind: ReferenceBindingKind::CallOutput {
                                            point: id,
                                            output: *output,
                                        },
                                    },
                                    kind: RuntimeBackingKind::OwnedAllocation,
                                    origin: origin.clone(),
                                    length: length.clone(),
                                    callable_family: callable_family.clone(),
                                }),
                            };
                            if let Some(mut fact) = fact {
                                let owner = match candidate {
                                    RuntimeOutputBackingSource::Parameter(_) => fact.owner.clone(),
                                    RuntimeOutputBackingSource::Created { owner, .. } => {
                                        owner.clone()
                                    }
                                };
                                let output_binding = ReferenceBindingId {
                                    source: point.source_span.source.clone(),
                                    function_span: current_function_span,
                                    declaration_span: point.source_span.range,
                                    block_span: current_function_span,
                                    kind: ReferenceBindingKind::CallOutput {
                                        point: id,
                                        output: *output,
                                    },
                                };
                                let output_owner = match contract.projections.last() {
                                    Some(ReferencePlaceProjection::Field(
                                        ScalarFieldReference::Resolved(field),
                                    )) => ReferenceBindingId {
                                        kind: ReferenceBindingKind::Projected {
                                            field: field.index,
                                        },
                                        ..output_binding.clone()
                                    },
                                    Some(_) => unreachable!("typed aggregate backing field"),
                                    None => output_binding.clone(),
                                };
                                transfer_array_family(&mut fact, &owner, &output_owner, false);
                                fact.owner = output_owner;
                                if contract.projections.is_empty() {
                                    if !point.possible_backings.contains(&fact) {
                                        point.possible_backings.push(fact.clone());
                                    }
                                    let binding = state
                                        .backing_bindings
                                        .entry(fact.owner.clone())
                                        .or_default();
                                    if !binding.contains(&fact) {
                                        binding.push(fact);
                                    }
                                } else {
                                    let CfgPointKind::Call {
                                        expression:
                                            ScalarExpression::Call {
                                                name, name_span, ..
                                            },
                                        ..
                                    } = &original_points[id.0].kind
                                    else {
                                        unreachable!("typed aggregate call output")
                                    };
                                    let place = ReferencePlaceId {
                                        source: output_binding.source.clone(),
                                        function_span: output_binding.function_span,
                                        declaration_span: output_binding.declaration_span,
                                        block_span: output_binding.block_span,
                                        binding: output_binding,
                                        projections: contract.projections.clone(),
                                        place: ScalarPlace::Name {
                                            name: name.clone(),
                                            span: *name_span,
                                        },
                                    };
                                    let projected = RuntimeProjectedBackingFact {
                                        place,
                                        backing: fact,
                                    };
                                    if !point.possible_projected_backings.contains(&projected) {
                                        point.possible_projected_backings.push(projected.clone());
                                    }
                                    if !state.backing_projected.contains(&projected) {
                                        state.backing_projected.push(projected);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let CfgPointKind::Call {
                target:
                    ResolvedCallTarget::CallableField {
                        receiver_place,
                        field,
                    },
                output,
                argument_points,
                ..
            } = &point.kind
            {
                let mut place = receiver_place.clone();
                place.projections.push(ReferencePlaceProjection::Field(
                    ScalarFieldReference::Resolved(field.clone()),
                ));
                let projected = state
                    .callable_projected
                    .iter()
                    .filter(|candidate| reference_places_same(&candidate.place, &place))
                    .flat_map(|candidate| &candidate.environments);
                let paths = state
                    .backing_call_paths
                    .iter()
                    .find(|(span, _)| span == &point.source_span)
                    .map(|(_, paths)| paths.clone());
                let input_paths = match paths {
                    Some(paths) if !paths.is_empty() => paths,
                    _ => vec![Vec::new()],
                };
                for environment in projected {
                    for contract in literal_contracts.iter().filter(|contract| {
                        contract.literal == environment.literal
                            && contract.output == *output
                            && contract.runtime_array
                    }) {
                        for path in &input_paths {
                            for candidate in &contract.backings {
                                if let RuntimeOutputBackingSource::Parameter(position) = candidate {
                                    let symbolic = argument_points.get(*position).is_some_and(|id| {
                                        matches!(&original_points[id.0].kind, CfgPointKind::Read { binding } if cfg.backing_parameters.contains(binding))
                                    });
                                    if !symbolic
                                        && !path.iter().any(|input| input.position == *position)
                                    {
                                        errors.push(ReferenceAnalysisError::MissingContext {
                                            source_span: point.source_span.clone(),
                                            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                        });
                                    }
                                }
                            }
                            for candidate in &contract.backings {
                                let fact = match candidate {
                                    RuntimeOutputBackingSource::Parameter(position) => path
                                        .iter()
                                        .find(|input| input.position == *position)
                                        .map(|input| input.backing.clone()),
                                    RuntimeOutputBackingSource::Created {
                                        origin,
                                        action,
                                        lineage,
                                        length,
                                        callable_family,
                                        owner: _,
                                    } => Some(RuntimeBackingFact {
                                        value: RuntimeValueId {
                                            source: point.source_span.source.clone(),
                                            function_span: current_function_span,
                                            creation_span: point.source_span.range,
                                            position: RuntimeValuePosition::CallOutput {
                                                call: point.source_span.range,
                                                output: *output,
                                                target_source: contract.literal.source.clone(),
                                                target_declaration: contract.literal.range,
                                                origin: origin.clone(),
                                                action: action.clone(),
                                                lineage: Box::new(lineage.clone()),
                                            },
                                        },
                                        owner: ReferenceBindingId {
                                            source: point.source_span.source.clone(),
                                            function_span: current_function_span,
                                            declaration_span: point.source_span.range,
                                            block_span: current_function_span,
                                            kind: ReferenceBindingKind::CallOutput {
                                                point: id,
                                                output: *output,
                                            },
                                        },
                                        kind: RuntimeBackingKind::OwnedAllocation,
                                        origin: origin.clone(),
                                        length: length.clone(),
                                        callable_family: callable_family.clone(),
                                    }),
                                };
                                if let Some(mut fact) = fact {
                                    let owner = match candidate {
                                        RuntimeOutputBackingSource::Parameter(_) => {
                                            fact.owner.clone()
                                        }
                                        RuntimeOutputBackingSource::Created { owner, .. } => {
                                            owner.clone()
                                        }
                                    };
                                    let output_owner = ReferenceBindingId {
                                        source: point.source_span.source.clone(),
                                        function_span: current_function_span,
                                        declaration_span: point.source_span.range,
                                        block_span: current_function_span,
                                        kind: ReferenceBindingKind::CallOutput {
                                            point: id,
                                            output: *output,
                                        },
                                    };
                                    transfer_array_family(&mut fact, &owner, &output_owner, false);
                                    fact.owner = ReferenceBindingId {
                                        source: point.source_span.source.clone(),
                                        function_span: current_function_span,
                                        declaration_span: point.source_span.range,
                                        block_span: current_function_span,
                                        kind: ReferenceBindingKind::CallOutput {
                                            point: id,
                                            output: *output,
                                        },
                                    };
                                    if !point.possible_backings.contains(&fact) {
                                        point.possible_backings.push(fact.clone());
                                    }
                                    let binding = state
                                        .backing_bindings
                                        .entry(fact.owner.clone())
                                        .or_default();
                                    if !binding.contains(&fact) {
                                        binding.push(fact);
                                    }
                                }
                            }
                        }
                    }
                }
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
                let errors_before_release = errors.len();
                if matches!(point.kind, CfgPointKind::Release { .. }) {
                    for candidate in candidates {
                        if candidate.place.projections.is_empty() {
                            if let Some(backings) =
                                state.backing_bindings.get(&candidate.place.binding)
                            {
                                release_cleanup.extend(backings.iter().cloned());
                            }
                        }
                    }
                }
                for candidate in candidates {
                    let origins = conflicting_live_loan(
                        &original_points,
                        &donor_uses,
                        &cfg.facts,
                        &cfg.environments,
                        &state,
                        id,
                        &candidate.place,
                        ScalarReferenceMutability::Mutable,
                        AccessAuthority::Through(candidate.origin),
                    );
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins,
                        });
                    }
                    if matches!(point.kind, CfgPointKind::Release { .. }) {
                        let origins = live_captured_backing_origins(
                            &original_points,
                            &donor_uses,
                            &cfg.environments,
                            &state,
                            id,
                            &candidate.place.binding,
                        );
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
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
                if errors.len() != errors_before_release {
                    release_cleanup.clear();
                }
            }
            let mut released_values = Vec::new();
            for backing in &release_cleanup {
                if !released_values.contains(&backing.value) {
                    push_callable_array_cleanup(
                        &mut holder_actions,
                        &mut errors,
                        backing,
                        &point.source_span,
                    );
                    released_values.push(backing.value.clone());
                }
            }
            if let CfgPointKind::Release {
                target: ReleaseTarget::Raw { binding },
            } = &point.kind
            {
                let origins = live_captured_backing_origins(
                    &original_points,
                    &donor_uses,
                    &cfg.environments,
                    &state,
                    id,
                    binding,
                );
                if !origins.is_empty() {
                    errors.push(ReferenceAnalysisError::ConflictingPlace {
                        source_span: point.source_span.clone(),
                        origins,
                    });
                }
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
            assignment_frozen_flow::freeze(
                &mut state,
                &frozen_protocol,
                &cfg.evaluated_values.destinations,
                &cfg.facts,
                point,
                &mut errors,
            );
            let frozen_phase =
                match assignment_frozen_flow::phase_context(&state, &frozen_protocol, id) {
                    Ok(context) => context,
                    Err(reason) => {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: point.source_span.clone(),
                            reason,
                        });
                        continue;
                    }
                };
            let frozen_commit = assignment_frozen_flow::committed(
                &state,
                &cfg.evaluated_values.destinations,
                &cfg.evaluated_values.commits,
                id,
            );
            if let Some(commit) = cfg
                .evaluated_values
                .commits
                .iter()
                .find(|commit| commit.point == id)
            {
                let destination = cfg
                    .evaluated_values
                    .destinations
                    .iter()
                    .find(|destination| destination.id == commit.destination)
                    .expect("commit destination");
                if let CfgPointKind::Assign { target, .. } = &mut point.kind {
                    if let EvaluatedDestinationStorage::Direct(_) = destination.storage {
                        *target = assignment_frozen_flow::direct_place(
                            &state.assignment_destinations[&destination.point],
                        )
                        .clone();
                    }
                }
                if let CfgPointKind::AssignThrough { candidates, .. } = &mut point.kind {
                    candidates.clear();
                    for row in frozen_commit.as_ref().expect("registered through commit") {
                        let (origin, loan) = match &row.access {
                            FrozenAssignmentAccess::Through { origin, loan, .. }
                            | FrozenAssignmentAccess::ThroughIndexed { origin, loan, .. } => {
                                (*origin, loan)
                            }
                            _ => unreachable!("through frozen access"),
                        };
                        let candidate = InvalidationCandidate {
                            origin,
                            loan: loan.clone(),
                            place: row.place.clone(),
                        };
                        if !candidates.contains(&candidate) {
                            candidates.push(candidate);
                        }
                    }
                }
                state.selected_element = None;
            }
            let assignment_selection = if matches!(point.kind, CfgPointKind::Assign { .. }) {
                match &frozen_commit {
                    Some(rows) => assignment_frozen_flow::selection(rows),
                    None => state.selected_element.clone(),
                }
            } else {
                None
            };
            if let CfgPointKind::Assign {
                target,
                previous_origins,
                previous_loans,
                ..
            } = &mut point.kind
            {
                let previous_target = target;
                *previous_origins = stored_reference_origins(&state, previous_target);
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
                *previous_origins = candidates
                    .iter()
                    .flat_map(|candidate| stored_reference_origins(&state, &candidate.place))
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
            let mut borrow_output = BTreeSet::new();
            if let CfgPointKind::Borrow { request } = &point.kind {
                let input = match request {
                    ReferenceBorrowRequest::DirectStorage { .. } => {
                        ReferenceBorrowInput::DirectStorage
                    }
                    ReferenceBorrowRequest::Through { pointer_point, .. } => {
                        match state.point_values[pointer_point.0]
                            .as_ref()
                            .filter(|origins| !origins.is_empty())
                        {
                            Some(origins) => ReferenceBorrowInput::Through {
                                pointer_point: *pointer_point,
                                origins: origins.clone(),
                            },
                            None => ReferenceBorrowInput::MissingProducer {
                                pointer_point: *pointer_point,
                            },
                        }
                    }
                };
                if matches!(input, ReferenceBorrowInput::MissingProducer { .. }) {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    });
                }
                let function_span = original_points[0].source_span.range;
                let cases = evaluated_borrow::realize(
                    &mut cfg.facts,
                    &point,
                    function_span,
                    request,
                    &input,
                );
                let realized_cases = cases.clone();
                #[cfg(test)]
                observations.borrows.push(ReferenceBorrowTransferSnapshot {
                    point: id,
                    state: state.clone(),
                    input: input.clone(),
                    cases: cases.clone(),
                });
                let evaluation = validate_and_apply_reference_borrow_cases(
                    &cfg.producer_owner,
                    &cfg.facts,
                    &cfg.environments,
                    &original_points,
                    &donor_uses,
                    &mut state,
                    &point,
                    request,
                    input,
                    cases,
                    &mut errors,
                );
                borrow_output = evaluation.output.clone();
                if let Err(error) = name_resolution::initial_transfer::record(
                    &cfg.producer_owner, &original_points[id.0], function_span, &cfg.facts,
                    realized_cases, &evaluation, origin_receipts,
                ) {
                    errors.push(error);
                }
                if !cfg.borrow_evaluations.contains(&evaluation) {
                    cfg.borrow_evaluations.push(evaluation);
                }
            }
            if let CfgPointKind::ReadPlace {
                place,
                pointer_point: Some(pointer),
                ..
            } = &point.kind
            {
                for origin in state.point_values[pointer.0].iter().flatten() {
                    let loan = match &cfg.facts[origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => continue,
                    };
                    let mut target = loan.origin_place.clone();
                    target.place = place.clone();
                    collect_reference_projections(place, Some(loan), &mut target.projections);
                    let conflicts = conflicting_live_loan(
                        &original_points,
                        &donor_uses,
                        &cfg.facts,
                        &cfg.environments,
                        &state,
                        id,
                        &target,
                        ScalarReferenceMutability::Shared,
                        AccessAuthority::Through(*origin),
                    );
                    if !conflicts.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins: conflicts,
                        });
                    }
                }
            }
            if let CfgPointKind::Read { binding } = &point.kind {
                let mut conflicts = Vec::new();
                for origin in cfg
                    .environments
                    .iter()
                    .flat_map(|environment| &environment.loans)
                {
                    let loan = match &cfg.facts[origin.origin.0] {
                        ReferenceOrigin::Fresh { loan, .. }
                        | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                        ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => continue,
                    };
                    if loan.origin_place.binding == *binding {
                        conflicts.extend(live_callable_loan_conflicts(
                            &original_points,
                            &donor_uses,
                            &cfg.facts,
                            &cfg.environments,
                            &state,
                            id,
                            &loan.origin_place,
                            ScalarReferenceMutability::Shared,
                            AccessAuthority::DirectStorage,
                        ));
                    }
                }
                conflicts.dedup();
                if !conflicts.is_empty() {
                    errors.push(ReferenceAnalysisError::ConflictingPlace {
                        source_span: point.source_span.clone(),
                        origins: conflicts,
                    });
                }
            }
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
                if !place.projections.iter().any(|projection| {
                    matches!(projection, ReferencePlaceProjection::Dereference(_))
                }) {
                    let conflicts = live_callable_loan_conflicts(
                        &original_points,
                        &donor_uses,
                        &cfg.facts,
                        &cfg.environments,
                        &state,
                        id,
                        place,
                        ScalarReferenceMutability::Shared,
                        AccessAuthority::DirectStorage,
                    );
                    if !conflicts.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins: conflicts,
                        });
                    }
                }
            }
            if let CfgPointKind::Capture { captures, .. } = &point.kind {
                for capture in captures {
                    let place = ReferencePlaceId {
                        source: capture.binding.source.clone(),
                        function_span: capture.binding.function_span,
                        declaration_span: capture.binding.declaration_span,
                        block_span: capture.binding.block_span,
                        binding: capture.binding.clone(),
                        projections: Vec::new(),
                        place: capture.place.clone(),
                    };
                    if unavailable_place(&state, &place)
                        || state.released.contains(&capture.binding)
                        || state.invalidated.contains(&capture.binding)
                    {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: capture.use_span.clone(),
                            origins: unavailable_origins(&state, &place),
                        });
                    }
                    if let Some(origins) = state.bindings.get(&capture.binding) {
                        for origin in origins {
                            let loan = match &cfg.facts[origin.0] {
                                ReferenceOrigin::Fresh { loan, .. }
                                | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                    continue
                                }
                            };
                            if unavailable_place(&state, &loan.origin_place)
                                || state.released.contains(&loan.origin_place.binding)
                                || state.invalidated.contains(&loan.origin_place.binding)
                            {
                                errors.push(ReferenceAnalysisError::ConflictingPlace {
                                    source_span: capture.use_span.clone(),
                                    origins: vec![SourceSpan::new(
                                        loan.creation_source.clone(),
                                        loan.creation_span,
                                    )],
                                });
                            }
                        }
                    }
                }
            }
            match &point.kind {
                CfgPointKind::Move {
                    place,
                    ty:
                        ty @ (ScalarType::Struct(_)
                        | ScalarType::Array { .. }
                        | ScalarType::RuntimeArray { .. }
                        | ScalarType::CheckedReference {
                            mutability: ScalarReferenceMutability::Mutable,
                            ..
                        }),
                    ..
                }
                | CfgPointKind::DeferredMove { place, ty, .. }
                    if matches!(point.kind, CfgPointKind::Move { .. })
                        || deferred_policies.get(&id.0) == Some(&CopyPolicy::Move) =>
                {
                    let place = available_place(place);
                    let mut origins = unavailable_origins(&state, &place);
                    if matches!(ty, ScalarType::CheckedReference { .. }) {
                        for origin in state.value.iter().flatten() {
                            let loan = match &cfg.facts[origin.0] {
                                ReferenceOrigin::Fresh { loan, .. }
                                | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                    continue
                                }
                            };
                            origins.extend(conflicting_live_loan(
                                &original_points,
                                &donor_uses,
                                &cfg.facts,
                                &cfg.environments,
                                &state,
                                id,
                                &loan.origin_place,
                                ScalarReferenceMutability::Mutable,
                                AccessAuthority::Through(*origin),
                            ));
                        }
                    }
                    origins.extend(conflicting_live_loan(
                        &original_points,
                        &donor_uses,
                        &cfg.facts,
                        &cfg.environments,
                        &state,
                        id,
                        &place,
                        ScalarReferenceMutability::Mutable,
                        AccessAuthority::DirectStorage,
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
                            &donor_uses,
                            &cfg.facts,
                            &cfg.environments,
                            &state,
                            id,
                            &place,
                            ScalarReferenceMutability::Mutable,
                            AccessAuthority::Through(candidate.origin),
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
                                    candidate.loan.creation_source.clone(),
                                    candidate.loan.creation_span,
                                ),
                            ));
                        }
                    }
                }
                CfgPointKind::Assign { target, .. } => {
                    let target = available_place(target);
                    let origins = conflicting_live_loan(
                        &original_points,
                        &donor_uses,
                        &cfg.facts,
                        &cfg.environments,
                        &state,
                        id,
                        &target,
                        ScalarReferenceMutability::Mutable,
                        AccessAuthority::DirectStorage,
                    );
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins,
                        });
                    }
                    let selected = assignment_selection
                        .as_ref()
                        .map(|(_, place)| place)
                        .unwrap_or(&target);
                    if checked_install.is_none() || !checked_input.is_empty() {
                        state.moved.retain(|(moved, _)| {
                            moved.binding != selected.binding
                                || !moved.projections.starts_with(&selected.projections)
                        });
                    }
                }
                CfgPointKind::AssignThrough { candidates, .. } => {
                    for candidate in candidates {
                        let target = available_place(&candidate.place);
                        let origins = conflicting_live_loan(
                            &original_points,
                            &donor_uses,
                            &cfg.facts,
                            &cfg.environments,
                            &state,
                            id,
                            &target,
                            ScalarReferenceMutability::Mutable,
                            AccessAuthority::Through(candidate.origin),
                        );
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
                        if checked_install.is_none() || !checked_input.is_empty() {
                            state.moved.retain(|(moved, _)| {
                                moved.binding != target.binding
                                    || !moved.projections.starts_with(&target.projections)
                            });
                        }
                    }
                }
                _ => {}
            }
            if let CfgPointKind::AssignmentSnapshot {
                destination_point, ..
            } = &point.kind
            {
                let frozen_snapshot = state.assignment_destinations[destination_point].clone();
                assignment_saved_flow::capture(
                    &mut state,
                    point,
                    &frozen_snapshot,
                    frozen_phase.as_ref().expect("actual snapshot phase"),
                    &mut errors,
                );
            }
            let callable_value = match &point.kind {
                _ if control_value => state.callable_point_values[id.0].clone(),
                CfgPointKind::AssignmentSnapshot { .. } | CfgPointKind::AssignmentResult { .. } => {
                    assignment_saved_flow::publish(
                        &mut state,
                        &original_points,
                        point,
                        frozen_phase.as_ref().expect("actual saved phase"),
                    );
                    point.possible_environments.clone()
                }
                CfgPointKind::AssignmentDispose { snapshot_point } => {
                    assignment_saved_flow::dispose(
                        &mut state,
                        *snapshot_point,
                        frozen_phase.as_ref().expect("actual disposal phase"),
                    );
                    BTreeSet::new()
                }
                CfgPointKind::Capture {
                    environment,
                    captures,
                    ..
                } => {
                    let fact = &mut cfg.environments[environment.index];
                    let mut path = Vec::new();
                    for capture in captures {
                        fact.owners.insert(capture.binding.clone());
                        let input_index = match fact
                            .inputs
                            .iter()
                            .position(|input| input.binding == capture.binding)
                        {
                            Some(index) => index,
                            None => {
                                let index = fact.inputs.len();
                                fact.inputs.push(CallableCaptureInput {
                                    binding: capture.binding.clone(),
                                    origins: BTreeSet::new(),
                                    environments: BTreeSet::new(),
                                    projected: Vec::new(),
                                    backings: Vec::new(),
                                    projected_backings: Vec::new(),
                                });
                                index
                            }
                        };
                        let input = &mut fact.inputs[input_index];
                        let captured_backings = match state
                            .backing_bindings
                            .get_mut(&capture.binding)
                        {
                            Some(backings) => {
                                for backing in backings.iter_mut() {
                                    if !matches!(
                                        backing.owner.kind,
                                        ReferenceBindingKind::Captured { .. }
                                    ) {
                                        let owner = ReferenceBindingId {
                                            source: environment.literal.source.clone(),
                                            function_span: capture.binding.function_span,
                                            declaration_span: capture.binding.declaration_span,
                                            block_span: capture.binding.block_span,
                                            kind: ReferenceBindingKind::Captured {
                                                environment_start: environment.literal.range.start,
                                                environment_end: environment.literal.range.end,
                                            },
                                        };
                                        transfer_array_family(
                                            backing,
                                            &capture.binding,
                                            &owner,
                                            false,
                                        );
                                        backing.owner = owner;
                                    }
                                }
                                backings.clone()
                            }
                            None => Vec::new(),
                        };
                        if matches!(
                            capture.ty,
                            ScalarType::RuntimeArray { .. } | ScalarType::Array { .. }
                        ) && captured_backings.is_empty()
                        {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: capture.use_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                            });
                        }
                        for backing in &captured_backings {
                            if !input.backings.contains(backing) {
                                input.backings.push(backing.clone());
                            }
                            if !fact.backings.contains(backing) {
                                fact.backings.push(backing.clone());
                            }
                        }
                        let mut capture_place = ReferencePlaceId {
                            source: capture.binding.source.clone(),
                            function_span: capture.binding.function_span,
                            declaration_span: capture.binding.declaration_span,
                            block_span: capture.binding.block_span,
                            binding: capture.binding.clone(),
                            projections: Vec::new(),
                            place: capture.place.clone(),
                        };
                        collect_reference_projections(
                            &capture.place,
                            None,
                            &mut capture_place.projections,
                        );
                        let projected_backings = state
                            .backing_projected
                            .iter_mut()
                            .filter(|known| reference_places_overlap(&known.place, &capture_place))
                            .map(|known| {
                                if !matches!(
                                    known.backing.owner.kind,
                                    ReferenceBindingKind::Captured { .. }
                                ) {
                                    let owner = ReferenceBindingId {
                                        source: environment.literal.source.clone(),
                                        function_span: capture.binding.function_span,
                                        declaration_span: known.backing.owner.declaration_span,
                                        block_span: capture.binding.block_span,
                                        kind: ReferenceBindingKind::Captured {
                                            environment_start: environment.literal.range.start,
                                            environment_end: environment.literal.range.end,
                                        },
                                    };
                                    let previous_owner = known.backing.owner.clone();
                                    transfer_array_family(
                                        &mut known.backing,
                                        &previous_owner,
                                        &owner,
                                        false,
                                    );
                                    known.backing.owner = owner;
                                }
                                known.clone()
                            })
                            .collect::<Vec<_>>();
                        for projected in &projected_backings {
                            if !input.projected_backings.contains(projected) {
                                input.projected_backings.push(projected.clone());
                            }
                            if !fact.backings.contains(&projected.backing) {
                                fact.backings.push(projected.backing.clone());
                            }
                        }
                        if let Some(origins) = state.bindings.get(&capture.binding) {
                            fact.origins.extend(origins);
                            input.origins.extend(origins);
                            fact.loans.extend(
                                origins
                                    .iter()
                                    .filter(|origin| {
                                        matches!(
                                            cfg.facts[origin.0],
                                            ReferenceOrigin::Fresh { .. }
                                                | ReferenceOrigin::BorrowedFrom { .. }
                                        )
                                    })
                                    .map(|origin| CallableLoanId { origin: *origin }),
                            );
                        }
                        if let Some(environments) = state.callable_bindings.get(&capture.binding) {
                            input.environments.extend(environments.iter().cloned());
                        }
                        for projected in &state.callable_projected {
                            if projected.place.binding == capture.binding
                                && reference_places_overlap(
                                    &projected.place,
                                    &ReferencePlaceId {
                                        source: capture.binding.source.clone(),
                                        function_span: capture.binding.function_span,
                                        declaration_span: capture.binding.declaration_span,
                                        block_span: capture.binding.block_span,
                                        binding: capture.binding.clone(),
                                        projections: Vec::new(),
                                        place: capture.place.clone(),
                                    },
                                )
                            {
                                if let Some(known) = input.projected.iter_mut().find(|known| {
                                    reference_places_same(&known.place, &projected.place)
                                }) {
                                    known
                                        .environments
                                        .extend(projected.environments.iter().cloned());
                                } else {
                                    input.projected.push(projected.clone());
                                }
                            }
                        }
                        for projected in &state.projected {
                            if projected.place.binding == capture.binding
                                && reference_places_overlap(
                                    &projected.place,
                                    &ReferencePlaceId {
                                        source: capture.binding.source.clone(),
                                        function_span: capture.binding.function_span,
                                        declaration_span: capture.binding.declaration_span,
                                        block_span: capture.binding.block_span,
                                        binding: capture.binding.clone(),
                                        projections: Vec::new(),
                                        place: capture.place.clone(),
                                    },
                                )
                            {
                                fact.origins.extend(&projected.origins);
                                fact.loans.extend(
                                    projected
                                        .origins
                                        .iter()
                                        .filter(|origin| {
                                            matches!(
                                                cfg.facts[origin.0],
                                                ReferenceOrigin::Fresh { .. }
                                                    | ReferenceOrigin::BorrowedFrom { .. }
                                            )
                                        })
                                        .map(|origin| CallableLoanId { origin: *origin }),
                                );
                            }
                        }
                        let mut snapshot = input.clone();
                        snapshot.backings = captured_backings;
                        snapshot.projected_backings = projected_backings;
                        snapshot.origins = match state.bindings.get(&capture.binding) {
                            Some(origins) => origins.clone(),
                            None => BTreeSet::new(),
                        };
                        snapshot.environments = match state.callable_bindings.get(&capture.binding)
                        {
                            Some(environments) => environments.clone(),
                            None => BTreeSet::new(),
                        };
                        snapshot.projected = state
                            .callable_projected
                            .iter()
                            .filter(|known| known.place.binding == capture.binding)
                            .cloned()
                            .collect();
                        path.push(snapshot);
                    }
                    if path
                        .iter()
                        .any(|input| !input.environments.is_empty() || !input.projected.is_empty())
                    {
                        pending_captures.push(PendingCapturePath {
                            capture: id,
                            span: point.source_span.clone(),
                            captures: captures.clone(),
                            path: path.clone(),
                            provenance: state.holder_provenance.clone(),
                        });
                    }
                    if !fact.capture_paths.contains(&path) {
                        fact.capture_paths.push(path);
                    }
                    BTreeSet::from([environment.clone()])
                }
                CfgPointKind::Read { binding } => {
                    state.callable_value_projected = state
                        .callable_projected
                        .iter()
                        .filter(|known| known.place.binding == *binding)
                        .cloned()
                        .collect();
                    state
                        .callable_bindings
                        .get(binding)
                        .cloned()
                        .unwrap_or_default()
                }
                CfgPointKind::ReadPlace { candidates, .. } => {
                    if aggregate_read {
                        state.callable_value_projected = state.callable_projected.iter().filter(|known|
                            candidates.iter().any(|candidate| {
                                let selected = available_place(candidate);
                                binding_holder::field_relative(&known.place, &selected.binding, &selected.projections).is_some()
                            })).cloned().collect();
                    }
                    candidates
                    .iter()
                    .flat_map(|candidate| {
                        let place = available_place(candidate);
                        let mut environments = state
                            .callable_projected
                            .iter()
                            .filter(|known| reference_places_same(&known.place, &place))
                            .flat_map(|known| known.environments.iter().cloned())
                            .collect::<BTreeSet<_>>();
                        if place.projections.is_empty() {
                            environments.extend(
                                state
                                    .callable_bindings
                                    .get(&place.binding)
                                    .into_iter()
                                    .flat_map(|known| known.iter().cloned()),
                            );
                        }
                        if let Some(ReferencePlaceProjection::Dereference(loan)) =
                            candidate.projections.last()
                        {
                            environments.extend(
                                indexed_loan_cells(&state, loan).into_iter().filter_map(
                                    |(_, _, case, _)| match case {
                                        CallableCellCaseFact::Initialized {
                                            environment_origin,
                                            ..
                                        } => Some(environment_origin),
                                        CallableCellCaseFact::Uninitialized => None,
                                    },
                                ),
                            );
                        }
                        environments
                    })
                    .collect()
                },
                CfgPointKind::Bind { binding } if parameter_points.contains(&id) => {
                    parameter_input::callable(&state, binding)
                }
                CfgPointKind::Bind { binding } => {
                    let environments = if let Some(input) = &binding_input {
                        state.callable_point_values[input.source.point.0].clone()
                    } else {
                        state.callable_value.clone()
                    };
                    state
                        .callable_bindings
                        .insert(binding.clone(), environments.clone());
                    environments
                }
                CfgPointKind::BindArrayElement { target, rhs_point } => {
                    let environments = state.callable_point_values[rhs_point.0].clone();
                    state
                        .callable_projected
                        .retain(|known| !reference_places_overlap(&known.place, target));
                    if !environments.is_empty() {
                        state.callable_projected.push(CallableProjectedEnvironment {
                            place: target.clone(),
                            environments: environments.clone(),
                        });
                    }
                    environments
                }
                CfgPointKind::Assign {
                    target, rhs_point, ..
                } => {
                    let environments = state.callable_point_values[rhs_point.0].clone();
                    let projected_target = assignment_selection
                        .as_ref()
                        .map(|(_, place)| place)
                        .unwrap_or(target);
                    if target.projections.is_empty() {
                        state
                            .callable_bindings
                            .insert(target.binding.clone(), environments.clone());
                        state
                            .callable_projected
                            .retain(|known| known.place.binding != target.binding);
                    } else if !(target.projections.iter().any(|projection| matches!(projection, ReferencePlaceProjection::Index(expression) if !matches!(expression, ScalarExpression::Integer { .. })))
                        && state.backing_bindings.get(&target.binding).is_some_and(|facts| facts.iter().any(|fact| fact.kind == RuntimeBackingKind::OwnedAllocation && !fact.callable_family.is_empty()))) {
                        state
                            .callable_projected
                            .retain(|known| !reference_places_same(&known.place, projected_target));
                        if !environments.is_empty() {
                            state.callable_projected.push(CallableProjectedEnvironment {
                                place: projected_target.clone(),
                                environments: environments.clone(),
                            });
                        }
                    }
                    environments
                }
                CfgPointKind::ReturnOutput { value_point, .. } => {
                    let mut environments = state.callable_point_values[value_point.0].clone();
                    point.possible_projected_environments =
                        state.callable_point_projected[value_point.0].clone();
                    if let CfgPointKind::Read { binding } = &original_points[value_point.0].kind {
                        point.possible_projected_environments = state
                            .callable_projected
                            .iter()
                            .filter(|known| known.place.binding == *binding)
                            .cloned()
                            .collect();
                    }
                    environments.extend(
                        point
                            .possible_projected_environments
                            .iter()
                            .flat_map(|known| known.environments.iter().cloned()),
                    );
                    let mut expired = Vec::new();
                    for environment in &environments {
                        let fact = &cfg.environments[environment.index];
                        for loan_id in &fact.loans {
                            let origin = &cfg.facts[loan_id.origin.0];
                            if let ReferenceOrigin::BorrowedFrom { loan, .. } = origin {
                                if cfg
                                    .reference_parameters
                                    .contains(&loan.origin_place.binding)
                                {
                                    continue;
                                }
                            }
                            let (loan, caller_owned) = match origin {
                                ReferenceOrigin::Fresh { loan, .. } => (loan, false),
                                ReferenceOrigin::BorrowedFrom { loan, .. } => (loan, true),
                                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                    continue
                                }
                            };
                            if caller_owned || !fact.owners.contains(&loan.origin_place.binding) {
                                let source = SourceSpan::new(
                                    loan.creation_source.clone(),
                                    loan.creation_span,
                                );
                                if !expired.contains(&source) {
                                    expired.push(source);
                                }
                            }
                        }
                    }
                    if !expired.is_empty() {
                        errors.push(ReferenceAnalysisError::EscapingCapture {
                            source_span: point.source_span.clone(),
                            origins: expired,
                        });
                    }
                    environments
                }
                CfgPointKind::Call { .. } => point.possible_environments.clone(),
                CfgPointKind::Join => state.callable_value.clone(),
                CfgPointKind::Evaluate(_) if point.possible_origins.is_none() => {
                    state.callable_value.clone()
                }
                _ => BTreeSet::new(),
            };
            point.possible_environments = callable_value.clone();
            if matches!(
                point.kind,
                CfgPointKind::Capture { .. }
                    | CfgPointKind::Read { .. }
                    | CfgPointKind::ReadPlace { .. }
                    | CfgPointKind::Bind { .. }
                    | CfgPointKind::BindArrayElement { .. }
                    | CfgPointKind::Assign { .. }
                    | CfgPointKind::Evaluate(_)
                    | CfgPointKind::Call { .. }
                    | CfgPointKind::Join
                    | CfgPointKind::AssignmentSnapshot { .. }
                     | CfgPointKind::AssignmentResult { .. }
                     | CfgPointKind::ValueOutputProjection { .. }
            ) {
                state.callable_value = callable_value;
            }
            state.callable_point_values[id.0] = point.possible_environments.clone();
            if let CfgPointKind::Bind { binding } = &point.kind {
                if parameter_points.contains(&id) {
                    point.possible_projected_environments = state.callable_projected.iter()
                        .filter(|input| input.place.binding == *binding).cloned().collect();
                    state.callable_point_projected[id.0] = point.possible_projected_environments.clone();
                    state.callable_value_projected = point.possible_projected_environments.clone();
                }
            }
            if let Some(input) = &binding_input {
                point.possible_projected_environments =
                    state.callable_point_projected[input.source.point.0].clone();
                state.callable_point_projected[id.0] =
                    point.possible_projected_environments.clone();
            } else if matches!(point.kind, CfgPointKind::Call { .. }) {
                state.callable_point_projected[id.0] =
                    point.possible_projected_environments.clone();
            }
            if control_value {
                point.possible_projected_environments = state.callable_point_projected[id.0].clone();
                state.callable_value_projected = point.possible_projected_environments.clone();
            } else if aggregate_read || matches!(point.kind, CfgPointKind::Read { .. } | CfgPointKind::Join) {
                state.callable_point_projected[id.0] = state.callable_value_projected.clone();
            }
            match &point.kind {
                _ if control_value => {
                    state.holder_value = state.holder_point_values[id.0].clone();
                    state.holder_sources = state.holder_point_sources[id.0].clone();
                }
                CfgPointKind::AssignmentSnapshot { .. } | CfgPointKind::AssignmentResult { .. } => {
                }
                CfgPointKind::AssignmentDispose { .. } => {
                    state.holder_value.clear();
                    state.holder_sources.clear();
                }
                CfgPointKind::Capture { .. } | CfgPointKind::Call { .. } => {
                    for environment in &point.possible_environments {
                        if point
                            .possible_projected_environments
                            .iter()
                            .any(|projection| projection.environments.contains(environment))
                        {
                            continue;
                        }
                        state.holder_relations.push(CallableHolderRelation {
                            environment: environment.clone(),
                            holders: vec![CallableHolderPlace::Temporary(id)],
                            capture_paths: cfg.environments[environment.index]
                                .capture_paths
                                .clone(),
                        });
                    }
                    for projection in &point.possible_projected_environments {
                        for environment in &projection.environments {
                            state.holder_relations.push(CallableHolderRelation {
                                environment: environment.clone(),
                                holders: vec![CallableHolderPlace::Projection(
                                    projection.place.clone(),
                                )],
                                capture_paths: cfg.environments[environment.index]
                                    .capture_paths
                                    .clone(),
                            });
                        }
                    }
                    normalize_holder_relations(&mut state.holder_relations);
                    record_holder_creation_roots(
                        &point,
                        id,
                        &state.holder_relations,
                        &mut state.holder_provenance,
                    );
                    state.holder_value = state
                        .holder_relations
                        .iter()
                        .filter(|relation| relation.holders.contains(&CallableHolderPlace::Temporary(id))
                            || relation.holders.iter().any(|holder| matches!(holder, CallableHolderPlace::Projection(place) if matches!(place.binding.kind, ReferenceBindingKind::CallOutput { point, .. } if point == id))))
                        .cloned()
                        .collect();
                    state.holder_sources = state.holder_value.iter()
                        .flat_map(|relation| relation.holders.iter())
                        .filter(|holder| match holder {
                            CallableHolderPlace::Temporary(created) => *created == id,
                            CallableHolderPlace::Projection(place) => matches!(place.binding.kind,
                                ReferenceBindingKind::CallOutput { point: created, .. } if created == id),
                            _ => false,
                        })
                        .cloned().collect();
                }
                CfgPointKind::Read { binding } => {
                    state.holder_value = state
                        .holder_relations
                        .iter()
                        .filter(|relation| {
                            relation.holders.iter().any(|holder| match holder {
                                CallableHolderPlace::Binding(known) => known == binding,
                                CallableHolderPlace::Projection(place) => &place.binding == binding,
                                _ => false,
                            })
                        })
                        .cloned()
                        .collect();
                    state.holder_sources = state
                        .holder_value
                        .iter()
                        .flat_map(|relation| relation.holders.iter())
                        .filter(|holder| match holder {
                            CallableHolderPlace::Binding(known) => known == binding,
                            CallableHolderPlace::Projection(place) => &place.binding == binding,
                            _ => false,
                        })
                        .cloned()
                        .collect();
                }
                CfgPointKind::ReadPlace { candidates, .. } => {
                    let sources = candidates
                        .iter()
                        .map(|candidate| {
                            let place = available_place(candidate);
                            if place.projections.is_empty() {
                                CallableHolderPlace::Binding(place.binding)
                            } else {
                                CallableHolderPlace::Projection(place)
                            }
                        })
                        .collect::<Vec<_>>();
                    let checked_dereference = candidates.iter().any(|candidate| {
                        candidate.projections.iter().any(|projection| {
                            matches!(projection, ReferencePlaceProjection::Dereference(_))
                        })
                    });
                    if checked_dereference {
                        for candidate in candidates {
                            if let Some(ReferencePlaceProjection::Dereference(loan)) =
                                candidate.projections.last()
                            {
                                for (holder, guard, case, _) in indexed_loan_cells(&state, loan) {
                                    if let CallableCellCaseFact::Initialized {
                                        environment_origin,
                                        capture_paths,
                                        ..
                                    } = &case
                                    {
                                        let action = CallableHolderAction::RetainCheckedSelected {
                                            holder,
                                            old: case.clone(),
                                            guard,
                                            to: CallableHolderPlace::Temporary(id),
                                            span: point.source_span.clone(),
                                        };
                                        holder_actions.push(action.clone());
                                        state.holder_relations.push(CallableHolderRelation {
                                            environment: environment_origin.clone(),
                                            holders: vec![CallableHolderPlace::Temporary(id)],
                                            capture_paths: capture_paths.clone(),
                                        });
                                        state.holder_provenance.insert(
                                            environment_origin.clone(),
                                            CallableHolderPlace::Temporary(id),
                                            HolderProducerRef::InstalledRead {
                                                point: id,
                                                environment: environment_origin.clone(),
                                                action,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        for source in &sources {
                            transfer_holder(
                                &mut state.holder_relations,
                                source,
                                CallableHolderPlace::Temporary(id),
                                false,
                                &point.source_span,
                                &mut holder_actions,
                                id,
                                &mut state.holder_provenance,
                            );
                        }
                        normalize_holder_relations(&mut state.holder_relations);
                        state.holder_value = state
                            .holder_relations
                            .iter()
                            .filter(|relation| {
                                relation
                                    .holders
                                    .contains(&CallableHolderPlace::Temporary(id))
                            })
                            .cloned()
                            .collect();
                        state.holder_sources = vec![CallableHolderPlace::Temporary(id)];
                    } else {
                        state.holder_value = state
                            .holder_relations
                            .iter()
                            .filter(|relation| {
                                relation
                                    .holders
                                    .iter()
                                    .any(|holder| sources.contains(holder))
                            })
                            .cloned()
                            .collect();
                        state.holder_sources = sources;
                        for candidate in candidates {
                            if !candidate.projections.iter().any(|projection| {
                                matches!(projection, ReferencePlaceProjection::Index(_))
                            }) {
                                continue;
                            }
                            if let Some(backings) = state.backing_bindings.get(&candidate.binding) {
                                for backing in backings.iter().filter(|backing| {
                                    backing.kind == RuntimeBackingKind::OwnedAllocation
                                }) {
                                    for family in &backing.callable_family {
                                        for partition in &family.partitions {
                                            for case in partition
                                                .representatives
                                                .iter()
                                                .flat_map(|representative| {
                                                    &representative.cell.alternatives
                                                })
                                                .chain(&partition.remainder.alternatives)
                                            {
                                                let CallableCellCaseFact::Initialized {
                                                    environment_origin,
                                                    capture_paths,
                                                    ..
                                                } = case
                                                else {
                                                    continue;
                                                };
                                                let holder = CallableArraySelectedHolderFact {
                                                    value: backing.value.clone(),
                                                    owner: backing.owner.clone(),
                                                    length: backing.length.clone(),
                                                    index_point: id,
                                                    occurrence: point.source_span.clone(),
                                                    place: candidate.clone(),
                                                };
                                                let action =
                                                    CallableHolderAction::RetainCheckedSelected {
                                                        holder,
                                                        old: case.clone(),
                                                        guard: partition.relations.clone(),
                                                        to: CallableHolderPlace::Temporary(id),
                                                        span: point.source_span.clone(),
                                                    };
                                                if !holder_actions.contains(&action) {
                                                    holder_actions.push(action.clone());
                                                }
                                                state.holder_provenance.insert(
                                                    environment_origin.clone(),
                                                    CallableHolderPlace::Temporary(id),
                                                    HolderProducerRef::InstalledRead {
                                                        point: id,
                                                        environment: environment_origin.clone(),
                                                        action,
                                                    },
                                                );
                                                let relation = CallableHolderRelation {
                                                    environment: environment_origin.clone(),
                                                    holders: vec![CallableHolderPlace::Temporary(
                                                        id,
                                                    )],
                                                    capture_paths: capture_paths.clone(),
                                                };
                                                if !state.holder_relations.contains(&relation) {
                                                    state.holder_relations.push(relation);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        normalize_holder_relations(&mut state.holder_relations);
                        state.holder_value.extend(
                            state
                                .holder_relations
                                .iter()
                                .filter(|relation| {
                                    relation
                                        .holders
                                        .contains(&CallableHolderPlace::Temporary(id))
                                })
                                .cloned(),
                        );
                        if !state.holder_value.is_empty() {
                            state
                                .holder_sources
                                .push(CallableHolderPlace::Temporary(id));
                        }
                    }
                }
                CfgPointKind::Bind { binding } if parameter_points.contains(&id) => {
                    parameter_input::holders(&mut state, binding);
                }
                CfgPointKind::Bind { binding } => {
                    let (holder_values, holder_sources, source_points) =
                        if let Some(input) = &binding_input {
                            (
                                state.holder_point_values[input.source.point.0].clone(),
                                state.holder_point_sources[input.source.point.0].clone(),
                                vec![&original_points[input.source.point.0]],
                            )
                        } else {
                            (
                                state.holder_value.clone(),
                                state.holder_sources.clone(),
                                original_points
                                    .iter()
                                    .filter(|candidate| candidate.successors.contains(&id))
                                    .collect::<Vec<_>>(),
                            )
                        };
                    if !original_points[id.0].successors.iter().any(|next| {
                        matches!(&original_points[next.0].kind, CfgPointKind::BindArrayElement { target, .. } if target.binding == *binding)
                    }) {
                    let field_read = source_points.iter().filter_map(|candidate| match &candidate.kind {
                        CfgPointKind::ReadPlace { candidates, .. } if candidates.len() == 1
                            && candidates[0].projections.iter().any(|projection| matches!(projection, ReferencePlaceProjection::Field(_))) => Some(CallableHolderPlace::Projection(available_place(&candidates[0]))),
                        _ => None,
                    }).find(|source| holder_sources.iter().any(|holder| holder_place_same(holder, source)));
                    if let Some(source) = field_read {
                        let source_binding = match &source {
                            CallableHolderPlace::Binding(binding) => Some(binding.clone()),
                            CallableHolderPlace::Projection(place) => {
                                Some(place.binding.clone())
                            }
                            _ => None,
                        };
                        let uninitialized_binding = source_binding.filter(|binding| {
                            state.uninitialized.contains(binding)
                        });
                        if let Some(source_binding) = uninitialized_binding {
                            let read_span = original_points
                                .iter()
                                .filter(|candidate| candidate.successors.contains(&id))
                                .find_map(|candidate| match &candidate.kind {
                                    CfgPointKind::ReadPlace { .. } => {
                                        Some(candidate.source_span.clone())
                                    }
                                    _ => None,
                                })
                                .unwrap_or_else(|| point.source_span.clone());
                            errors.push(ReferenceAnalysisError::MissingOrigin {
                                source_span: read_span,
                                binding_id: source_binding,
                                reason:
                                    ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        } else {
                            let mut donors = Vec::new();
                            for relation in holder_values.iter().filter(|relation| relation.holders.iter().any(|holder| holder_place_same(holder, &source))) {
                                let donor = if let Some(input) = &binding_input {
                                    match binding_holder::donor(&original_points, first_binding_input.as_ref(), &state, input, &source, relation) {
                                        Ok(donor) => donor,
                                        Err(error) => { errors.push(error); continue; }
                                    }
                                } else { source.clone() };
                                if !donors.iter().any(|known| holder_place_same(known, &donor)) { donors.push(donor); }
                            }
                            for donor in donors {
                            transfer_holder(
                                &mut state.holder_relations,
                                &donor,
                                CallableHolderPlace::Binding(binding.clone()),
                                false,
                                &point.source_span,
                                &mut holder_actions,
                                id,
                                &mut state.holder_provenance,
                            );
                            }
                        }
                    } else {
                    replicate_global_struct_holders(
                        &source_points,
                        &mut state,
                        binding,
                        &point.source_span,
                        &mut holder_actions,
                        id,
                        &mut errors,
                    );
                    let value = holder_values
                        .iter()
                        .filter(|relation| {
                            point.possible_environments.contains(&relation.environment)
                        })
                        .filter(|relation| {
                            !holder_values.iter().any(|projected| {
                                projected.environment == relation.environment
                                    && projected.holders.iter().any(|holder| {
                                        matches!(holder, CallableHolderPlace::Projection(_))
                                    })
                            })
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                     let mut donors = Vec::new();
                     for relation in value {
                         for source in holder_sources.iter().filter(|source| {
                            relation.holders.contains(source)
                                && matches!(
                                    source,
                                    CallableHolderPlace::Temporary(_)
                                        | CallableHolderPlace::Binding(_)
                                )
                        }) {
                            let donor = if let Some(input) = &binding_input {
                                 match binding_holder::donor(&original_points, first_binding_input.as_ref(), &state, input, source, &relation) {
                                    Ok(donor) => donor,
                                    Err(error) => { errors.push(error); continue; }
                                }
                             } else { source.clone() };
                             if !donors.iter().any(|known| holder_place_same(known, &donor)) { donors.push(donor); }
                         }
                     }
                     for donor in donors {
                             transfer_holder(
                                &mut state.holder_relations,
                                &donor,
                                CallableHolderPlace::Binding(binding.clone()),
                                false,
                                &point.source_span,
                                &mut holder_actions,
                                id,
                                &mut state.holder_provenance,
                            );
                     }
                    }
                    }
                }
                CfgPointKind::BindArrayElement { target, rhs_point } => {
                    let source_holder = match &original_points[rhs_point.0].kind {
                        CfgPointKind::Read { binding } => {
                            CallableHolderPlace::Binding(binding.clone())
                        }
                        CfgPointKind::ReadPlace { candidates, .. } if candidates.len() == 1 => {
                            CallableHolderPlace::Projection(candidates[0].clone())
                        }
                        _ => CallableHolderPlace::Temporary(*rhs_point),
                    };
                    transfer_holder(
                        &mut state.holder_relations,
                        &source_holder,
                        CallableHolderPlace::Projection(target.clone()),
                        false,
                        &point.source_span,
                        &mut holder_actions,
                        id,
                        &mut state.holder_provenance,
                    );
                }
                CfgPointKind::Assign {
                    target, rhs_point, ..
                } => {
                    let selected = assignment_selection.clone();
                    let symbolic = selected.as_ref().is_some_and(|(_, place)| {
                        place.projections.iter().any(|projection| {
                            matches!(projection, ReferencePlaceProjection::Index(_))
                        }) && state
                            .backing_bindings
                            .get(&place.binding)
                            .is_some_and(|facts| {
                                facts.iter().any(|fact| {
                                    fact.kind == RuntimeBackingKind::OwnedAllocation
                                        && !fact.callable_family.is_empty()
                                })
                            })
                    });
                    if symbolic {
                        let (index_point, selected_place) =
                            selected.as_ref().expect("selected runtime element");
                        let source = selected_place
                            .projections
                            .iter()
                            .rev()
                            .find_map(|projection| match projection {
                                ReferencePlaceProjection::Index(expression) => {
                                    Some(SourceSpan::new(
                                        selected_place.source.clone(),
                                        expression_span(expression),
                                    ))
                                }
                                _ => None,
                            })
                            .expect("typed selected index");
                        let source_holder = match &original_points[rhs_point.0].kind {
                            CfgPointKind::Read { binding } => {
                                CallableHolderPlace::Binding(binding.clone())
                            }
                            CfgPointKind::ReadPlace { candidates, .. } if candidates.len() == 1 => {
                                CallableHolderPlace::Projection(candidates[0].clone())
                            }
                            _ => CallableHolderPlace::Temporary(*rhs_point),
                        };
                        let incoming = state.holder_point_values[rhs_point.0]
                            .iter()
                            .filter(|relation| relation.holders.contains(&source_holder))
                            .map(|relation| {
                                let mut aliases = relation
                                    .holders
                                    .iter()
                                    .filter(|holder| {
                                        !matches!(holder, CallableHolderPlace::Temporary(_))
                                    })
                                    .cloned()
                                    .collect::<Vec<_>>();
                                aliases
                                    .push(CallableHolderPlace::Projection(selected_place.clone()));
                                aliases.sort_by(holder_place_cmp);
                                aliases.dedup_by(|left, right| holder_place_same(left, right));
                                CallableArrayElementFact {
                                    place: selected_place.clone(),
                                    index: match selected_place
                                        .projections
                                        .iter()
                                        .rev()
                                        .find(|projection| {
                                            matches!(projection, ReferencePlaceProjection::Index(_))
                                        })
                                        .expect("typed indexed place")
                                    {
                                        ReferencePlaceProjection::Index(
                                            ScalarExpression::Integer { value, .. },
                                        ) => CallableArrayIndexFact::Constant(value.clone()),
                                        ReferencePlaceProjection::Index(_) => {
                                            CallableArrayIndexFact::Selected {
                                                point: *index_point,
                                                occurrence: source.clone(),
                                                function_span: selected_place.function_span,
                                            }
                                        }
                                        _ => unreachable!("typed indexed place"),
                                    },
                                    environment: relation.environment.clone(),
                                    instances: if selected_place.projections.iter().any(
                                        |projection| {
                                            matches!(
                                                projection,
                                                ReferencePlaceProjection::Index(
                                                    ScalarExpression::Integer { .. }
                                                )
                                            )
                                        },
                                    ) {
                                        CallableArrayEnvironmentInstances::KnownElement
                                    } else {
                                        CallableArrayEnvironmentInstances::PerEvaluatedIndex
                                    },
                                    aliases,
                                    loans: cfg.environments[relation.environment.index]
                                        .loans
                                        .clone(),
                                }
                            })
                            .collect::<Vec<_>>();
                        holder_actions.push(CallableHolderAction::SelectElement {
                            target: target.clone(),
                            index_point: *index_point,
                            selected: selected_place.clone(),
                        });
                        if incoming.is_empty() {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                            });
                        }
                        if let Some(backings) =
                            state.backing_bindings.get_mut(&selected_place.binding)
                        {
                            for backing in backings {
                                for family in &mut backing.callable_family {
                                    let holder = CallableArraySelectedHolderFact {
                                        value: backing.value.clone(),
                                        owner: backing.owner.clone(),
                                        length: backing.length.clone(),
                                        index_point: *index_point,
                                        occurrence: source.clone(),
                                        place: selected_place.clone(),
                                    };
                                    let key = match selected_place
                                        .projections
                                        .iter()
                                        .rev()
                                        .find(|projection| {
                                            matches!(projection, ReferencePlaceProjection::Index(_))
                                        })
                                        .expect("typed indexed place")
                                    {
                                        ReferencePlaceProjection::Index(
                                            ScalarExpression::Integer { value, .. },
                                        ) => CallableIndexKeyFact::Constant(value.clone()),
                                        ReferencePlaceProjection::Index(_) => {
                                            CallableIndexKeyFact::Evaluated {
                                                point: *index_point,
                                                occurrence: source.clone(),
                                                function_span: selected_place.function_span,
                                            }
                                        }
                                        _ => unreachable!("typed indexed place"),
                                    };
                                    for partition in &family.partitions {
                                        let selected_cell = &partition
                                            .representatives
                                            .iter()
                                            .find(|representative| representative.key == key)
                                            .expect("evaluated selected index representative")
                                            .cell;
                                        for case in &selected_cell.alternatives {
                                            match case {
                                                CallableCellCaseFact::Uninitialized => {
                                                    holder_actions.push(CallableHolderAction::SkipUninitializedSelected {
                                                        holder: holder.clone(),
                                                        guard: partition.relations.clone(),
                                                        span: point.source_span.clone(),
                                                    });
                                                }
                                                CallableCellCaseFact::Initialized { .. } => {
                                                    holder_actions.push(
                                                        CallableHolderAction::DropSelected {
                                                            holder: holder.clone(),
                                                            previous: vec![
                                                                selected_previous_element(
                                                                    case,
                                                                    partition,
                                                                    &key,
                                                                    selected_place,
                                                                    *index_point,
                                                                    &source,
                                                                ),
                                                            ],
                                                            old: case.clone(),
                                                            guard: partition.relations.clone(),
                                                            span: point.source_span.clone(),
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    let mut installed = Vec::new();
                                    for element in &incoming {
                                        let action = if matches!(
                                            source_holder,
                                            CallableHolderPlace::Temporary(_)
                                                | CallableHolderPlace::Projection(
                                                    ReferencePlaceId {
                                                        binding: ReferenceBindingId {
                                                            kind:
                                                                ReferenceBindingKind::CallOutput { .. },
                                                            ..
                                                        },
                                                        ..
                                                    }
                                                )
                                        ) {
                                            CallableHolderAction::MoveSelected {
                                                environment: element.environment.clone(),
                                                from: source_holder.clone(),
                                                to: holder.clone(),
                                                span: point.source_span.clone(),
                                            }
                                        } else {
                                            CallableHolderAction::RetainSelected {
                                                environment: element.environment.clone(),
                                                from: source_holder.clone(),
                                                to: holder.clone(),
                                                span: point.source_span.clone(),
                                            }
                                        };
                                        holder_actions.push(action);
                                        if !family.elements.contains(element) {
                                            family.elements.push(element.clone());
                                        }
                                        let case = CallableCellCaseFact::Initialized {
                                            environment_origin: element.environment.clone(),
                                            instance:
                                                CallableEnvironmentInstanceClassFact::FreshPerCell,
                                            holder_aliases: element.aliases.clone(),
                                            backing: backing.value.clone(),
                                            owner: backing.owner.clone(),
                                            capture_paths: cfg.environments
                                                [element.environment.index]
                                                .capture_paths
                                                .clone(),
                                            loans: element.loans.clone(),
                                            moved: false,
                                            origin: element.environment.literal.clone(),
                                        };
                                        if !installed.contains(&case) {
                                            installed.push(case);
                                        }
                                    }
                                    for partition in &mut family.partitions {
                                        let equal = partition
                                            .relations
                                            .iter()
                                            .filter_map(|relation| {
                                                if relation.relation
                                                    != CallableArrayIndexRelation::Equal
                                                {
                                                    return None;
                                                }
                                                if relation.left == key {
                                                    Some(relation.right.clone())
                                                } else if relation.right == key {
                                                    Some(relation.left.clone())
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect::<Vec<_>>();
                                        for representative in &mut partition.representatives {
                                            if representative.key == key
                                                || equal.contains(&representative.key)
                                            {
                                                representative.cell.alternatives =
                                                    installed.clone();
                                            }
                                        }
                                    }
                                    fold_indexed_representative(family, &key);
                                    normalize_array_family(family);
                                }
                            }
                        }
                        if selected_place.projections.iter().any(|projection| {
                            matches!(
                                projection,
                                ReferencePlaceProjection::Index(ScalarExpression::Integer { .. })
                            )
                        }) {
                            let target_holder =
                                CallableHolderPlace::Projection(selected_place.clone());
                            if source_holder != target_holder {
                                let prior_actions = holder_actions.len();
                                remove_holder(
                                    &mut state.holder_relations,
                                    &target_holder,
                                    &point.source_span,
                                    &mut holder_actions,
                                    &mut state.holder_provenance,
                                );
                                holder_actions.truncate(prior_actions);
                                transfer_holder(
                                    &mut state.holder_relations,
                                    &source_holder,
                                    target_holder,
                                    false,
                                    &point.source_span,
                                    &mut holder_actions,
                                    id,
                                    &mut state.holder_provenance,
                                );
                            }
                        }
                    } else {
                        if let Some((index_point, selected_place)) = &selected {
                            holder_actions.push(CallableHolderAction::SelectElement {
                                target: target.clone(),
                                index_point: *index_point,
                                selected: selected_place.clone(),
                            });
                        }
                        let target_holder = if let Some((_, selected_place)) = selected {
                            CallableHolderPlace::Projection(selected_place)
                        } else if target.projections.is_empty() {
                            CallableHolderPlace::Binding(target.binding.clone())
                        } else {
                            CallableHolderPlace::Projection(target.clone())
                        };
                        let source_holder = match &original_points[rhs_point.0].kind {
                            CfgPointKind::AssignmentSnapshot { .. }
                            | CfgPointKind::AssignmentResult { .. }
                                if !state.holder_point_sources[rhs_point.0].is_empty() =>
                            {
                                assignment_saved_flow::root_holder(&state, *rhs_point)
                            }
                            CfgPointKind::Read { binding } => {
                                CallableHolderPlace::Binding(binding.clone())
                            }
                            CfgPointKind::ReadPlace { candidates, .. } if candidates.len() == 1 => {
                                CallableHolderPlace::Projection(candidates[0].clone())
                            }
                            _ => CallableHolderPlace::Temporary(*rhs_point),
                        };
                        if source_holder != target_holder {
                            remove_holder(
                                &mut state.holder_relations,
                                &target_holder,
                                &point.source_span,
                                &mut holder_actions,
                                &mut state.holder_provenance,
                            );
                            transfer_holder(
                                &mut state.holder_relations,
                                &source_holder,
                                target_holder,
                                false,
                                &point.source_span,
                                &mut holder_actions,
                                id,
                                &mut state.holder_provenance,
                            );
                        }
                    }
                }
                CfgPointKind::AssignThrough {
                    candidates,
                    rhs_point,
                    ..
                } => {
                    let source_holder = match &original_points[rhs_point.0].kind {
                        CfgPointKind::AssignmentSnapshot { .. }
                        | CfgPointKind::AssignmentResult { .. }
                            if !state.holder_point_sources[rhs_point.0].is_empty() =>
                        {
                            assignment_saved_flow::root_holder(&state, *rhs_point)
                        }
                        CfgPointKind::Read { binding } => {
                            CallableHolderPlace::Binding(binding.clone())
                        }
                        CfgPointKind::ReadPlace { candidates, .. } if candidates.len() == 1 => {
                            CallableHolderPlace::Projection(candidates[0].clone())
                        }
                        _ => CallableHolderPlace::Temporary(*rhs_point),
                    };
                    for candidate in candidates {
                        let place = available_place(&candidate.place);
                        if state
                            .backing_bindings
                            .get(&place.binding)
                            .is_some_and(|backings| {
                                backings.iter().any(|backing| {
                                    backing.kind == RuntimeBackingKind::OwnedAllocation
                                        && !backing.callable_family.is_empty()
                                })
                            })
                            && place.projections.iter().any(|projection| {
                                matches!(projection, ReferencePlaceProjection::Index(_))
                            })
                        {
                            let old_cells = indexed_loan_cells(&state, &candidate.loan);
                            if old_cells.is_empty() {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                    source_span: point.source_span.clone(),
                                    reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                });
                            }
                            for (holder, guard, old, key) in old_cells {
                                holder_actions.push(CallableHolderAction::SelectElement {
                                    target: candidate.place.clone(),
                                    index_point: holder.index_point,
                                    selected: place.clone(),
                                });
                                match old {
                                    CallableCellCaseFact::Uninitialized => holder_actions.push(CallableHolderAction::SkipUninitializedSelected {
                                        holder: holder.clone(), guard, span: point.source_span.clone(),
                                    }),
                                    CallableCellCaseFact::Initialized { .. } => holder_actions.push(CallableHolderAction::DropSelected {
                                        previous: vec![CallableArrayElementFact {
                                            place: place.clone(),
                                            index: CallableArrayIndexFact::Selected { point: holder.index_point, occurrence: holder.occurrence.clone(), function_span: place.function_span },
                                            environment: match &old { CallableCellCaseFact::Initialized { environment_origin, .. } => environment_origin.clone(), _ => unreachable!() },
                                            instances: CallableArrayEnvironmentInstances::PerEvaluatedIndex,
                                            aliases: match &old { CallableCellCaseFact::Initialized { holder_aliases, .. } => holder_aliases.clone(), _ => unreachable!() },
                                            loans: match &old { CallableCellCaseFact::Initialized { loans, .. } => loans.clone(), _ => unreachable!() },
                                        }],
                                        holder: holder.clone(), old, guard, span: point.source_span.clone(),
                                    }),
                                }
                                let incoming = state.holder_point_values[rhs_point.0]
                                    .iter()
                                    .filter(|relation| relation.holders.contains(&source_holder))
                                    .cloned()
                                    .collect::<Vec<_>>();
                                if incoming.is_empty() {
                                    errors.push(ReferenceAnalysisError::MissingContext {
                                        source_span: point.source_span.clone(),
                                        reason:
                                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                    });
                                }
                                for relation in &incoming {
                                    let action = if matches!(
                                        source_holder,
                                        CallableHolderPlace::Temporary(_)
                                    ) {
                                        CallableHolderAction::MoveSelected {
                                            environment: relation.environment.clone(),
                                            from: source_holder.clone(),
                                            to: holder.clone(),
                                            span: point.source_span.clone(),
                                        }
                                    } else {
                                        CallableHolderAction::RetainSelected {
                                            environment: relation.environment.clone(),
                                            from: source_holder.clone(),
                                            to: holder.clone(),
                                            span: point.source_span.clone(),
                                        }
                                    };
                                    if !holder_actions.contains(&action) {
                                        holder_actions.push(action);
                                    }
                                }
                                if let Some(backings) =
                                    state.backing_bindings.get_mut(&place.binding)
                                {
                                    for backing in backings.iter_mut().filter(|backing| {
                                        backing.value == holder.value
                                            && backing.owner == holder.owner
                                            && backing.length == holder.length
                                    }) {
                                        for family in &mut backing.callable_family {
                                            for partition in &mut family.partitions {
                                                let equal = partition
                                                    .relations
                                                    .iter()
                                                    .filter_map(|relation| {
                                                        if relation.relation
                                                            != CallableArrayIndexRelation::Equal
                                                        {
                                                            return None;
                                                        }
                                                        if relation.left == key {
                                                            Some(relation.right.clone())
                                                        } else if relation.right == key {
                                                            Some(relation.left.clone())
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                    .collect::<Vec<_>>();
                                                let installed = incoming.iter().map(|relation| CallableCellCaseFact::Initialized {
                                                    environment_origin: relation.environment.clone(),
                                                    instance: CallableEnvironmentInstanceClassFact::FreshPerCell,
                                                    holder_aliases: vec![CallableHolderPlace::Projection(place.clone())],
                                                    backing: backing.value.clone(), owner: backing.owner.clone(),
                                                    capture_paths: relation.capture_paths.clone(),
                                                    loans: cfg.environments[relation.environment.index].loans.clone(),
                                                    moved: false, origin: relation.environment.literal.clone(),
                                                }).collect::<Vec<_>>();
                                                for representative in &mut partition.representatives
                                                {
                                                    if representative.key == key
                                                        || equal.contains(&representative.key)
                                                    {
                                                        representative.cell.alternatives =
                                                            installed.clone();
                                                    }
                                                }
                                            }
                                            normalize_indexed_partitions(family);
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        let target = if place.projections.is_empty() {
                            CallableHolderPlace::Binding(place.binding.clone())
                        } else {
                            CallableHolderPlace::Projection(place)
                        };
                        if source_holder != target {
                            remove_holder(
                                &mut state.holder_relations,
                                &target,
                                &point.source_span,
                                &mut holder_actions,
                                &mut state.holder_provenance,
                            );
                            transfer_holder(
                                &mut state.holder_relations,
                                &source_holder,
                                target,
                                false,
                                &point.source_span,
                                &mut holder_actions,
                                id,
                                &mut state.holder_provenance,
                            );
                        }
                    }
                }
                CfgPointKind::CopyCallableFields { source, target } => {
                    let source: &ReferenceBindingId = match source {
                        CallableFieldSource::Assignment(source) => source,
                        CallableFieldSource::Evaluated(_) => {
                            let Some(donor) = &field_donor else {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                    source_span: point.source_span.clone(),
                                    reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                                });
                                continue;
                            };
                            donor
                        }
                    };
                    let moved = matches!(source.kind, ReferenceBindingKind::CallOutput { .. })
                        || state
                            .moved
                            .iter()
                            .any(|(place, _)| place.binding == *source);
                    let (_, _, first) = field_route.as_ref().expect("actual copy event");
                    let relations = match &field_receiving {
                        Some(input) if matches!(original_points[input.source.point.0].kind,
                            CfgPointKind::Read { .. } | CfgPointKind::ReadPlace { .. } | CfgPointKind::Call { .. }) =>
                            state.holder_point_values[input.source.point.0].clone(),
                        _ => state.holder_relations.clone(),
                    };
                    let mut copies = Vec::new();
                    let errors_before_fields = errors.len();
                    for relation in &relations {
                        for holder in &relation.holders {
                            let CallableHolderPlace::Projection(place) = holder else { continue; };
                            let Some(relative) = binding_holder::field_relative(place, source, &field_prefix) else { continue; };
                            let donor = match binding_holder::field_donor(&original_points, id, *first, &state, place, &relative, relation) {
                                Ok(donor) => donor,
                                Err(error) => { errors.push(error); continue; }
                            };
                            if !copies.iter().any(|(known, _)| holder_place_same(known, &donor)) { copies.push((donor, relative)); }
                        }
                    }
                    if errors.len() != errors_before_fields { continue; }
                    for (from, mut new_place) in copies {
                        new_place.source = target.source.clone();
                        new_place.function_span = target.function_span;
                        new_place.declaration_span = target.declaration_span;
                        new_place.block_span = target.block_span;
                        new_place.binding = target.clone();
                        let to = CallableHolderPlace::Projection(new_place);
                        remove_holder(
                            &mut state.holder_relations,
                            &to,
                            &point.source_span,
                            &mut holder_actions,
                            &mut state.holder_provenance,
                        );
                        transfer_holder(
                            &mut state.holder_relations,
                            &from,
                            to,
                            moved && first.is_none(),
                            &point.source_span,
                            &mut holder_actions,
                            id,
                            &mut state.holder_provenance,
                        );
                    }
                }
                CfgPointKind::ReturnOutput {
                    output,
                    value_point,
                    ..
                } => {
                    let sources = state.holder_point_sources[value_point.0].clone();
                    let returned = if matches!(
                        original_points[value_point.0].kind,
                        CfgPointKind::AssignmentSnapshot { .. }
                            | CfgPointKind::AssignmentResult { .. }
                    ) {
                        state
                            .holder_relations
                            .iter()
                            .filter(|relation| {
                                relation
                                    .holders
                                    .iter()
                                    .any(|holder| sources.contains(holder))
                            })
                            .cloned()
                            .collect()
                    } else {
                        state.holder_point_values[value_point.0].clone()
                    };
                    for relation in returned {
                        for holder in relation
                            .holders
                            .into_iter()
                            .filter(|holder| sources.contains(holder))
                        {
                            let position = state.holder_relations.iter().position(|known| {
                                known.environment == relation.environment
                                    && known.holders.contains(&holder)
                            });
                            if let Some(position) = position {
                                let output_holder = match &holder {
                                    CallableHolderPlace::Projection(place) => {
                                        CallableHolderPlace::OutputProjection {
                                            output: *output,
                                            projections: place.projections.clone(),
                                        }
                                    }
                                    _ => CallableHolderPlace::Output(*output),
                                };
                                state.holder_relations[position]
                                    .holders
                                    .retain(|known| known != &holder);
                                state.holder_relations[position]
                                    .holders
                                    .push(output_holder.clone());
                                let producer = state
                                    .holder_provenance
                                    .lookup(&relation.environment, &holder)
                                    .cloned();
                                state
                                    .holder_provenance
                                    .remove(&relation.environment, &holder);
                                let transfer = CallableHolderAction::ReturnTransfer {
                                    environment: relation.environment.clone(),
                                    from: holder.clone(),
                                    to: output_holder,
                                    span: point.source_span.clone(),
                                };
                                holder_actions.push(transfer.clone());
                                if let Some(producer) = producer {
                                    pending_returns.push(PendingReturnCase {
                                        return_point: id,
                                        return_span: point.source_span.clone(),
                                        output: *output,
                                        holder,
                                        environment: relation.environment.clone(),
                                        transfer,
                                        producer,
                                    });
                                } else {
                                    let gap = PendingReturnGap {
                                        return_point: id,
                                        return_span: point.source_span.clone(),
                                        environment: relation.environment.clone(),
                                    };
                                    if !pending_return_gaps.iter().any(|known| {
                                        known.return_point == gap.return_point
                                            && known.environment == gap.environment
                                    }) {
                                        pending_return_gaps.push(gap);
                                    }
                                }
                            }
                        }
                    }
                    normalize_holder_relations(&mut state.holder_relations);
                }
                CfgPointKind::ScopeExit => {
                    let mut retired_cells = Vec::new();
                    let preserved = match conditional_output_production::forward_exit_holders(&cfg.evaluated_values, &cfg.producer_owner,
                        &original_points[id.0].successors, &mut state, id, &mut holder_actions) {
                        Ok(preserved) => preserved,
                        Err(error) => { errors.push(error); continue; }
                    };
                    let mut retired_values = Vec::new();
                    for capture in original_points
                        .iter()
                        .filter_map(|candidate| match &candidate.kind {
                            CfgPointKind::Capture { captures, .. } => Some(captures.as_slice()),
                            _ => None,
                        })
                        .flatten()
                    {
                        if capture.binding.block_span == point.source_span.range
                            && capture_value_cell_droppable(&capture.ty)
                            && cfg.top_level.points.is_empty()
                            && !state.backing_bindings.contains_key(&capture.binding)
                            && !retired_values.contains(&capture.binding)
                        {
                            retired_values.push(capture.binding.clone());
                            holder_actions.push(CallableHolderAction::DropCapturedValueCellOwner {
                                binding: capture.binding.clone(),
                                cell: SourceSpan::new(
                                    capture.binding.source.clone(),
                                    capture.binding.declaration_span,
                                ),
                                ty: capture.ty.clone(),
                                span: point.source_span.clone(),
                            });
                        }
                    }
                    for (binding, backings) in &state.backing_bindings {
                        if binding.block_span != point.source_span.range
                            || !matches!(binding.kind, ReferenceBindingKind::Declared)
                            || !cfg.top_level.points.is_empty()
                            || state.released.contains(binding)
                            || state.moved.iter().any(|(place, _)| {
                                place.binding == *binding && place.projections.is_empty()
                            })
                        {
                            continue;
                        }
                        let promoted = cfg.environments.iter().any(|environment| {
                            environment
                                .inputs
                                .iter()
                                .any(|input| input.binding == *binding)
                        });
                        if !promoted {
                            continue;
                        }
                        for backing in backings {
                            if retired_cells.contains(&(binding.clone(), backing.value.clone())) {
                                continue;
                            }
                            retired_cells.push((binding.clone(), backing.value.clone()));
                            holder_actions.push(CallableHolderAction::DropCaptureCellOwner {
                                binding: binding.clone(),
                                cell: SourceSpan::new(
                                    binding.source.clone(),
                                    binding.declaration_span,
                                ),
                                backing: backing.clone(),
                                span: point.source_span.clone(),
                            });
                        }
                    }
                    let mut exiting = Vec::new();
                    for (binding, backings) in &state.backing_bindings {
                        if binding.block_span == point.source_span.range
                            && !state.released.contains(binding)
                        {
                            exiting.extend(backings.iter().cloned());
                        }
                    }
                    for projected in &state.backing_projected {
                        if projected.place.binding.block_span == point.source_span.range
                            && !state.released.contains(&projected.place.binding)
                        {
                            exiting.push(projected.backing.clone());
                        }
                    }
                    let mut walked = Vec::new();
                    for backing in exiting {
                        if !state.transferred_backings.contains(&backing.value)
                            && !retired_cells
                                .iter()
                                .any(|(_, value)| *value == backing.value)
                            && !walked.contains(&backing.value)
                        {
                            push_callable_array_cleanup(
                                &mut holder_actions,
                                &mut errors,
                                &backing,
                                &point.source_span,
                            );
                            walked.push(backing.value);
                        }
                    }
                    let departing = state.holder_relations.iter().flat_map(|relation| relation.holders.iter())
                        .filter(|holder| match holder {
                            CallableHolderPlace::Binding(binding) => binding.block_span == point.source_span.range,
                            CallableHolderPlace::Projection(place) => place.binding.block_span == point.source_span.range
                                || matches!(place.binding.kind, ReferenceBindingKind::CallOutput { point: created, .. } if original_points[created.0].scope == point.scope),
                            CallableHolderPlace::Temporary(created) => original_points[created.0].scope == point.scope,
                            CallableHolderPlace::Output(_) | CallableHolderPlace::OutputProjection { .. } => false,
                        }).cloned().collect::<Vec<_>>();
                    for holder in departing {
                        if preserved.contains(&holder) { continue; }
                        if original_points[id.0].successors.iter().any(|successor| {
                            matches!(original_points[successor.0].kind, CfgPointKind::Join)
                        }) && state.holder_sources.contains(&holder)
                        {
                            continue;
                        }
                        remove_holder(
                            &mut state.holder_relations,
                            &holder,
                            &point.source_span,
                            &mut holder_actions,
                            &mut state.holder_provenance,
                        );
                    }
                }
                _ => {}
            }
            if let CfgPointKind::Assign { target, .. } = &point.kind {
                if target.projections.is_empty() && assignment_selection.is_none() {
                    for backings in state.backing_bindings.values_mut() {
                        for backing in backings {
                            for family in &mut backing.callable_family {
                                let keys = family
                                    .partitions
                                    .iter()
                                    .flat_map(|partition| partition.representatives.iter())
                                    .filter_map(|representative| match &representative.key {
                                        CallableIndexKeyFact::LivePlace(place)
                                            if place.binding == target.binding =>
                                        {
                                            Some(representative.key.clone())
                                        }
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>();
                                for key in keys {
                                    fold_indexed_representative(family, &key);
                                }
                            }
                        }
                    }
                }
                let selected = match &assignment_selection {
                    Some((_, place)) => place,
                    None => target,
                };
                let incoming_elements =
                    callable_array_elements(&cfg.environments, &state, &selected.binding);
                if let Some(backings) = state.backing_bindings.get_mut(&selected.binding) {
                    for backing in backings {
                        let value = backing.value.clone();
                        let owner = backing.owner.clone();
                        for family in &mut backing.callable_family {
                            let changed_places = incoming_elements
                                .iter()
                                .filter(|element| reference_places_same(&element.place, selected))
                                .map(|element| element.place.clone())
                                .collect::<Vec<_>>();
                            family.elements.retain(|element| {
                                !changed_places
                                    .iter()
                                    .any(|place| reference_places_same(place, &element.place))
                            });
                            family.elements.extend(
                                incoming_elements
                                    .iter()
                                    .filter(|element| {
                                        changed_places.iter().any(|place| {
                                            reference_places_same(place, &element.place)
                                        })
                                    })
                                    .cloned(),
                            );
                            normalize_array_family(family);
                            for element in incoming_elements.iter().filter(|element| {
                                changed_places
                                    .iter()
                                    .any(|place| reference_places_same(place, &element.place))
                            }) {
                                record_known_indexed_element(
                                    family,
                                    element,
                                    &value,
                                    &owner,
                                    cfg.environments[element.environment.index]
                                        .capture_paths
                                        .clone(),
                                );
                            }
                        }
                    }
                }
            }
            if aggregate_read {
                let CfgPointKind::ReadPlace { candidates, .. } = &original_points[id.0].kind else { unreachable!("actual aggregate read"); };
                let mut relations = Vec::new();
                let mut sources = Vec::new();
                for relation in &state.holder_relations {
                    let mut selected = relation.clone();
                    selected.holders.retain(|holder| match holder {
                        CallableHolderPlace::Projection(place) => candidates.iter().any(|candidate| {
                            let source = available_place(candidate);
                            binding_holder::field_relative(place, &source.binding, &source.projections).is_some()
                        }),
                        _ => false,
                    });
                    if !selected.holders.is_empty() {
                        for holder in &selected.holders { if !sources.contains(holder) { sources.push(holder.clone()); } }
                        relations.push(selected);
                    }
                }
                state.holder_value = relations;
                state.holder_sources = sources;
            }
            state.holder_point_values[id.0] = state.holder_value.clone();
            state.holder_point_sources[id.0] = state.holder_sources.clone();
            if let CfgPointKind::CopyCallableFields { target, .. } = &point.kind {
                let (source, field_prefix, _) = field_route.as_ref().expect("actual checked field-copy route");
                let projected = state
                    .projected
                    .iter()
                    .filter(|known| binding_holder::field_relative(&known.place, source, &field_prefix).is_some())
                    .cloned()
                    .collect::<Vec<_>>();
                let moved = matches!(source.kind, ReferenceBindingKind::CallOutput { .. }) || state
                    .moved
                    .iter()
                    .any(|(place, _)| place.binding == *source && place.projections.is_empty());
                if moved {
                    state
                        .projected
                        .retain(|known| binding_holder::field_relative(&known.place, source, &field_prefix).is_none());
                }
                for mut known in projected {
                    known.place = binding_holder::field_relative(&known.place, source, &field_prefix).expect("selected checked-field prefix");
                    known.place.source = target.source.clone();
                    known.place.function_span = target.function_span;
                    known.place.declaration_span = target.declaration_span;
                    known.place.block_span = target.block_span;
                    known.place.binding = target.clone();
                    write_projected_reference(&mut state, &known.place, &known.origins);
                }
            }
            let checked_value = match &point.kind {
                _ if control_value => state.point_values[id.0].clone(),
                CfgPointKind::AssignmentSnapshot { .. } => {
                    Some(assignment_saved_flow::checked(&state, id))
                }
                CfgPointKind::AssignmentResult { snapshot_point, .. } => {
                    Some(assignment_saved_flow::checked(&state, *snapshot_point))
                }
                CfgPointKind::AssignmentDispose { snapshot_point } => {
                    let CfgPointKind::AssignmentSnapshot { holder, .. } =
                        &original_points[snapshot_point.0].kind
                    else {
                        unreachable!("assignment disposition snapshot")
                    };
                    state.bindings.remove(holder);
                    Some(BTreeSet::new())
                }
                CfgPointKind::ReturnOutput { value_point, .. } => {
                    checked_return_input(&state, *value_point)
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
                    read_checked_binding(&state, &cfg.checked_bindings, point, binding, &mut errors)
                }
                CfgPointKind::Bind { binding } if cfg.checked_bindings.contains_key(binding) => {
                    apply_checked_install(
                        &mut state,
                        &cfg.producer_owner,
                        &cfg.checked_bindings,
                        point,
                        frozen_commit.as_deref(),
                        &checked_install,
                        &checked_input,
                        &mut cfg.checked_install_evaluations,
                        &mut errors,
                    )
                }
                CfgPointKind::Assign { .. } | CfgPointKind::AssignThrough { .. } => {
                    apply_checked_install(
                        &mut state,
                        &cfg.producer_owner,
                        &cfg.checked_bindings,
                        point,
                        frozen_commit.as_deref(),
                        &checked_install,
                        &checked_input,
                        &mut cfg.checked_install_evaluations,
                        &mut errors,
                    )
                }
                CfgPointKind::Borrow { .. } => Some(borrow_output.clone()),
                CfgPointKind::Evaluate(_) | CfgPointKind::Call { .. } => {
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
            apply_checked_return(
                &mut state,
                &cfg.facts,
                &cfg.allocations,
                point,
                &checked_value,
                &returned_backings,
                &mut errors,
            );
            publish_checked_value(point, &checked_value);
            match &point.kind {
                CfgPointKind::Bind { binding } if parameter_points.contains(&id) => {
                    point.possible_backings = state.backing_bindings.get(binding)
                        .into_iter().flatten().cloned().collect();
                }
                CfgPointKind::Borrow { .. } => {
                    point.possible_backings = borrow_output
                        .iter()
                        .flat_map(|origin| match &cfg.facts[origin.0] {
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. } => state
                                .backing_bindings
                                .get(&loan.origin_place.binding)
                                .into_iter()
                                .flatten()
                                .cloned()
                                .collect::<Vec<_>>(),
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => {
                                Vec::new()
                            }
                        })
                        .collect();
                    state.value = checked_value.clone();
                }
                CfgPointKind::AssignmentSnapshot { .. } | CfgPointKind::AssignmentResult { .. } => {
                    state.value = checked_value.clone();
                }
                CfgPointKind::Read { binding } => {
                    point.possible_backings = state
                        .backing_bindings
                        .get(binding)
                        .into_iter()
                        .flatten()
                        .cloned()
                        .collect();
                    state.value = checked_value.clone();
                }
                CfgPointKind::Assign {
                    target, rhs_point, ..
                } if !target.projections.is_empty() && checked_value.is_none() => {
                    if let Some(source_facts) = state.backing_point_values.get(rhs_point).cloned() {
                        let field = match target.projections.last() {
                            Some(ReferencePlaceProjection::Field(
                                ScalarFieldReference::Resolved(field),
                            )) => field.index,
                            _ => unreachable!("typed runtime array field assignment"),
                        };
                        let previous = state
                            .backing_projected
                            .iter()
                            .filter(|known| reference_places_same(&known.place, target))
                            .map(|known| known.backing.clone())
                            .collect::<Vec<_>>();
                        for old in previous {
                            if !source_facts
                                .iter()
                                .any(|incoming| incoming.value == old.value)
                            {
                                push_callable_array_cleanup(
                                    &mut holder_actions,
                                    &mut errors,
                                    &old,
                                    &point.source_span,
                                );
                            }
                        }
                        state
                            .backing_projected
                            .retain(|known| !reference_places_same(&known.place, target));
                        for mut backing in source_facts {
                            let previous_owner = backing.owner.clone();
                            let owner = ReferenceBindingId {
                                source: target.source.clone(),
                                function_span: target.function_span,
                                declaration_span: target.declaration_span,
                                block_span: target.block_span,
                                kind: ReferenceBindingKind::Projected { field },
                            };
                            transfer_array_family(&mut backing, &previous_owner, &owner, false);
                            backing.owner = owner;
                            if state.moved.iter().any(|(moved, _)| {
                                moved.binding == previous_owner && moved.projections.is_empty()
                            }) {
                                state.backing_bindings.remove(&previous_owner);
                            }
                            state.backing_projected.push(RuntimeProjectedBackingFact {
                                place: target.clone(),
                                backing,
                            });
                        }
                    }
                }
                CfgPointKind::ReturnOutput {
                    value_point,
                    backing_action,
                    ..
                } => {
                    point.possible_backings = state
                        .backing_point_values
                        .get(value_point)
                        .into_iter()
                        .flatten()
                        .cloned()
                        .collect();
                    point.possible_projected_backings = match &original_points[value_point.0].kind {
                        CfgPointKind::Read { binding } => state
                            .backing_projected
                            .iter()
                            .filter(|known| known.place.binding == *binding)
                            .cloned()
                            .collect(),
                        CfgPointKind::ReadPlace { candidates, .. } => state
                            .backing_projected
                            .iter()
                            .filter_map(|known| {
                                candidates.iter().find_map(|candidate| {
                                    (known.place.binding == candidate.binding
                                        && known
                                            .place
                                            .projections
                                            .starts_with(&candidate.projections))
                                    .then(|| {
                                        let mut result = known.clone();
                                        result.place.projections = known.place.projections
                                            [candidate.projections.len()..]
                                            .to_vec();
                                        result
                                    })
                                })
                            })
                            .collect(),
                        CfgPointKind::Call { .. } => value_projected.clone(),
                        CfgPointKind::AssignmentSnapshot { .. }
                        | CfgPointKind::AssignmentResult { .. } => {
                            assignment_saved_flow::projected_backings(
                                &state,
                                &original_points,
                                *value_point,
                            )
                        }
                        _ => Vec::new(),
                    };
                    for environment in &point.possible_environments {
                        for input in &cfg.environments[environment.index].inputs {
                            if let Some(backings) = state.backing_bindings.get(&input.binding) {
                                for backing in backings {
                                    if matches!(
                                        backing.owner.kind,
                                        ReferenceBindingKind::Captured { .. }
                                    ) && !point.possible_backings.contains(backing)
                                    {
                                        point.possible_backings.push(backing.clone());
                                    }
                                }
                            }
                            for projected in state
                                .backing_projected
                                .iter()
                                .filter(|known| known.place.binding == input.binding)
                            {
                                if matches!(
                                    projected.backing.owner.kind,
                                    ReferenceBindingKind::Captured { .. }
                                ) && !point.possible_backings.contains(&projected.backing)
                                {
                                    point.possible_backings.push(projected.backing.clone());
                                }
                            }
                        }
                    }
                    if let RuntimeReturnBackingAction::CopyFixed { source } = backing_action {
                        for fact in &mut point.possible_backings {
                            if fact.owner == *source {
                                fact.value = RuntimeValueId {
                                    source: point.source_span.source.clone(),
                                    function_span: source.function_span,
                                    creation_span: point.source_span.range,
                                    position: RuntimeValuePosition::Local,
                                };
                                fact.kind = RuntimeBackingKind::OwnedAllocation;
                                fact.origin = point.source_span.clone();
                                if let RuntimeArrayLengthFact::Fixed { value, .. } = &fact.length {
                                    fact.length = RuntimeArrayLengthFact::Fixed {
                                        value: *value,
                                        origin: point.source_span.clone(),
                                    };
                                }
                            }
                        }
                    }
                    for backing in &point.possible_backings {
                        if backing.kind == RuntimeBackingKind::OwnedAllocation
                            && !state.transferred_backings.contains(&backing.value)
                        {
                            state.transferred_backings.push(backing.value.clone());
                        }
                    }
                    for projected in &point.possible_projected_backings {
                        if !state
                            .transferred_backings
                            .contains(&projected.backing.value)
                        {
                            state
                                .transferred_backings
                                .push(projected.backing.value.clone());
                        }
                    }
                    if let CfgPointKind::Read { binding } = &original_points[value_point.0].kind {
                        for projected in state
                            .backing_projected
                            .iter()
                            .filter(|projected| projected.place.binding == *binding)
                        {
                            if !state
                                .transferred_backings
                                .contains(&projected.backing.value)
                            {
                                state
                                    .transferred_backings
                                    .push(projected.backing.value.clone());
                            }
                        }
                    }
                }
                CfgPointKind::Backing {
                    target,
                    transfer,
                    materialize,
                } => {
                    let previous = state.backing_bindings.get(target).cloned();
                    let captured_owner = state
                        .backing_bindings
                        .get(target)
                        .and_then(|facts| {
                            facts.iter().find(|fact| {
                                matches!(fact.owner.kind, ReferenceBindingKind::Captured { .. })
                            })
                        })
                        .map(|fact| fact.owner.clone());
                    let target_kind = if *materialize {
                        RuntimeBackingKind::OwnedAllocation
                    } else {
                        RuntimeBackingKind::FixedValue
                    };
                    let facts = match transfer {
                        RuntimeBackingTransfer::Allocate | RuntimeBackingTransfer::CreateFixed => {
                            let length = cfg
                                .backing_length_sites
                                .iter()
                                .find(|site| site.binding == *target)
                                .cloned();
                            if length.is_none() {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                    source_span: point.source_span.clone(),
                                    reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                });
                            }
                            length
                                .into_iter()
                                .map(|site| RuntimeBackingFact {
                                    value: RuntimeValueId {
                                        source: point.source_span.source.clone(),
                                        function_span: target.function_span,
                                        creation_span: target.declaration_span,
                                        position: RuntimeValuePosition::Local,
                                    },
                                    owner: target.clone(),
                                    kind: target_kind.clone(),
                                    origin: point.source_span.clone(),
                                    length: site.length,
                                    callable_family: if matches!(
                                        site.element,
                                        ScalarType::Callable { .. }
                                    ) {
                                        vec![CallableArrayFamilyFact {
                                            element: site.element,
                                            elements: callable_array_elements(
                                                &cfg.environments,
                                                &state,
                                                target,
                                            ),
                                            index_relations: Vec::new(),
                                            unselected: CallableArrayUnselectedFact {
                                                selected: Vec::new(),
                                            },
                                            partitions: vec![CallableIndexedPartitionFact {
                                                representatives: Vec::new(),
                                                relations: Vec::new(),
                                                remainder: CallableCellDomainFact {
                                                    alternatives: vec![
                                                        CallableCellCaseFact::Uninitialized,
                                                    ],
                                                },
                                            }],
                                        }]
                                    } else {
                                        Vec::new()
                                    },
                                })
                                .map(|mut fact| {
                                    for family in &mut fact.callable_family {
                                        normalize_array_family(family);
                                    }
                                    seed_indexed_family(&mut fact, &cfg.environments);
                                    fact
                                })
                                .collect::<Vec<_>>()
                        }
                        RuntimeBackingTransfer::CopyFixed { source } => {
                            if !state.backing_bindings.contains_key(source) {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                    source_span: point.source_span.clone(),
                                    reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                });
                            }
                            state
                                .backing_bindings
                                .get(source)
                                .into_iter()
                                .flatten()
                                .filter_map(|source_fact| {
                                    let RuntimeArrayLengthFact::Fixed { value: length, .. } = &source_fact.length else {
                                        errors.push(ReferenceAnalysisError::MissingContext {
                                            source_span: point.source_span.clone(),
                                            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                        });
                                        return None;
                                    };
                                    let source_backing = source_fact.clone();
                                    Some((source_backing, RuntimeBackingFact {
                                    value: RuntimeValueId {
                                        source: point.source_span.source.clone(),
                                        function_span: target.function_span,
                                        creation_span: point.source_span.range,
                                        position: RuntimeValuePosition::Local,
                                    },
                                    owner: target.clone(),
                                    kind: target_kind.clone(),
                                    origin: point.source_span.clone(),
                                     length: RuntimeArrayLengthFact::Fixed {
                                        value: *length,
                                        origin: point.source_span.clone(),
                                    },
                                    callable_family: source_fact.callable_family.clone(),
                                }))})
                                 .map(|(source_backing, mut fact)| {
                                      transfer_array_family(&mut fact, source, target, true);
                                      if !fact.callable_family.is_empty() {
                                          let action = CallableHolderAction::RetainArrayElements {
                                              source: source_backing,
                                              backing: fact.clone(),
                                              span: point.source_span.clone(),
                                          };
                                          if !holder_actions.contains(&action) {
                                              holder_actions.push(action);
                                          }
                                      }
                                      fact
                                 })
                                 .collect()
                        }
                        RuntimeBackingTransfer::Move { source } => {
                            let source_facts = state.backing_bindings.remove(source);
                            if source_facts.is_none()
                                && !cfg.backing_parameters.contains(source)
                                && !matches!(source.kind, ReferenceBindingKind::CallOutput { .. })
                            {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                    source_span: point.source_span.clone(),
                                    reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                });
                            }
                            source_facts
                                .into_iter()
                                .flatten()
                                .map(|mut fact| {
                                    transfer_array_family(&mut fact, source, target, false);
                                    fact.owner = target.clone();
                                    fact.kind = target_kind.clone();
                                    fact
                                })
                                .collect()
                        }
                    };
                    let facts = facts
                        .into_iter()
                        .map(|mut fact| {
                            if let Some(owner) = &captured_owner {
                                transfer_array_family(&mut fact, target, owner, false);
                                fact.owner = owner.clone();
                            }
                            fact
                        })
                        .collect::<Vec<_>>();
                    if captured_owner.is_some() && previous.is_some() {
                        let previous = previous.expect("captured cell prior backing");
                        if previous.len() != 1 || facts.len() != 1 {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                            });
                        } else if validate_callable_array_cleanup(
                            &mut errors,
                            &previous[0],
                            &point.source_span,
                        ) && validate_callable_array_cleanup(
                            &mut errors,
                            &facts[0],
                            &point.source_span,
                        ) {
                            holder_actions.push(CallableHolderAction::ReplaceCaptureCellValue {
                                binding: target.clone(),
                                cell: SourceSpan::new(
                                    target.source.clone(),
                                    target.declaration_span,
                                ),
                                previous: previous[0].clone(),
                                incoming: facts[0].clone(),
                                span: point.source_span.clone(),
                            });
                        }
                    } else {
                        for old in previous.into_iter().flatten() {
                            if !facts.iter().any(|incoming| incoming.value == old.value)
                                && !state.released.contains(target)
                            {
                                push_callable_array_cleanup(
                                    &mut holder_actions,
                                    &mut errors,
                                    &old,
                                    &point.source_span,
                                );
                            }
                        }
                    }
                    state.backing_bindings.insert(target.clone(), facts.clone());
                    point.possible_backings = facts;
                    for relation in &mut state.holder_relations {
                        for path in &mut relation.capture_paths {
                            for input in path.iter_mut().filter(|input| input.binding == *target) {
                                input.backings = state.backing_bindings[target].clone();
                            }
                        }
                    }
                }
                CfgPointKind::BackingPlace { target, transfer } => {
                    let source = match transfer {
                        RuntimeBackingTransfer::CopyFixed { source }
                        | RuntimeBackingTransfer::Move { source } => source,
                        RuntimeBackingTransfer::Allocate | RuntimeBackingTransfer::CreateFixed => {
                            unreachable!("typed aggregate field input")
                        }
                    };
                    let source_facts = match transfer {
                        RuntimeBackingTransfer::CopyFixed { .. } => {
                            state.backing_bindings.get(source).cloned()
                        }
                        RuntimeBackingTransfer::Move { .. } => {
                            state.backing_bindings.remove(source)
                        }
                        RuntimeBackingTransfer::Allocate | RuntimeBackingTransfer::CreateFixed => {
                            unreachable!("typed aggregate field input")
                        }
                    };
                    let source_facts = match source_facts {
                        Some(facts) => facts,
                        None => {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                            });
                            Vec::new()
                        }
                    };
                    let field = match target.projections.last() {
                        Some(ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(
                            field,
                        ))) => field,
                        _ => unreachable!("typed aggregate field projection"),
                    };
                    let previous = state
                        .backing_projected
                        .iter()
                        .filter(|known| reference_places_same(&known.place, target))
                        .map(|known| known.backing.clone())
                        .collect::<Vec<_>>();
                    for old in previous {
                        if !source_facts
                            .iter()
                            .any(|incoming| incoming.value == old.value)
                        {
                            push_callable_array_cleanup(
                                &mut holder_actions,
                                &mut errors,
                                &old,
                                &point.source_span,
                            );
                        }
                    }
                    state
                        .backing_projected
                        .retain(|known| !reference_places_same(&known.place, target));
                    for mut backing in source_facts {
                        let source_backing = backing.clone();
                        if matches!(transfer, RuntimeBackingTransfer::CopyFixed { .. }) {
                            backing.value = RuntimeValueId {
                                source: point.source_span.source.clone(),
                                function_span: target.function_span,
                                creation_span: point.source_span.range,
                                position: RuntimeValuePosition::Local,
                            };
                            backing.origin = point.source_span.clone();
                        }
                        let owner = ReferenceBindingId {
                            source: target.source.clone(),
                            function_span: target.function_span,
                            declaration_span: point.source_span.range,
                            block_span: target.block_span,
                            kind: ReferenceBindingKind::Projected { field: field.index },
                        };
                        transfer_array_family(
                            &mut backing,
                            source,
                            &owner,
                            matches!(transfer, RuntimeBackingTransfer::CopyFixed { .. }),
                        );
                        backing.owner = owner;
                        backing.kind = RuntimeBackingKind::OwnedAllocation;
                        if matches!(transfer, RuntimeBackingTransfer::CopyFixed { .. })
                            && !backing.callable_family.is_empty()
                        {
                            let action = CallableHolderAction::RetainArrayElements {
                                source: source_backing,
                                backing: backing.clone(),
                                span: point.source_span.clone(),
                            };
                            if !holder_actions.contains(&action) {
                                holder_actions.push(action);
                            }
                        }
                        point.possible_backings.push(backing.clone());
                        state.backing_projected.push(RuntimeProjectedBackingFact {
                            place: target.clone(),
                            backing,
                        });
                    }
                }
                CfgPointKind::CopyProjectedBackings { source, target } => {
                    let moved = matches!(source.kind, ReferenceBindingKind::CallOutput { .. })
                        || state
                            .moved
                            .iter()
                            .any(|(place, _)| place.binding == *source);
                    let sources = state
                        .backing_projected
                        .iter()
                        .filter(|known| known.place.binding == *source)
                        .cloned()
                        .collect::<Vec<_>>();
                    if moved {
                        state
                            .backing_projected
                            .retain(|known| known.place.binding != *source);
                    }
                    for mut projected in sources {
                        let previous_owner = projected.backing.owner.clone();
                        let mut projections = target.projections.clone();
                        projections.extend(projected.place.projections);
                        let field = projections
                            .iter()
                            .rev()
                            .find_map(|projection| match projection {
                                ReferencePlaceProjection::Field(
                                    ScalarFieldReference::Resolved(field),
                                ) => Some(field.index),
                                _ => None,
                            })
                            .expect("typed nested array field");
                        projected.place = ReferencePlaceId {
                            source: target.source.clone(),
                            function_span: target.function_span,
                            declaration_span: target.declaration_span,
                            block_span: target.block_span,
                            binding: target.binding.clone(),
                            projections,
                            place: projected.place.place,
                        };
                        let owner = ReferenceBindingId {
                            source: target.source.clone(),
                            function_span: target.function_span,
                            declaration_span: target.declaration_span,
                            block_span: target.block_span,
                            kind: ReferenceBindingKind::Projected { field },
                        };
                        transfer_array_family(
                            &mut projected.backing,
                            &previous_owner,
                            &owner,
                            !moved,
                        );
                        projected.backing.owner = owner;
                        if !state.backing_projected.contains(&projected) {
                            state.backing_projected.push(projected);
                        }
                    }
                }
                CfgPointKind::CopyCallableFields { target, .. } => {
                    let (source, field_prefix, _) = field_route.as_ref().expect("actual backing field-copy route");
                    let moved = matches!(source.kind, ReferenceBindingKind::CallOutput { .. })
                        || state
                            .moved
                            .iter()
                             .any(|(place, _)| place.binding == *source);
                    let projected_backings = state
                        .backing_projected
                        .iter()
                        .filter(|known| binding_holder::field_relative(&known.place, source, &field_prefix).is_some())
                        .cloned()
                        .collect::<Vec<_>>();
                    if moved {
                        state
                            .backing_projected
                            .retain(|known| binding_holder::field_relative(&known.place, source, &field_prefix).is_none());
                    }
                    for mut known in projected_backings {
                        known.place = binding_holder::field_relative(&known.place, source, &field_prefix).expect("selected backing-field prefix");
                        known.place.binding = target.clone();
                        known.place.source = target.source.clone();
                        known.place.function_span = target.function_span;
                        known.place.declaration_span = target.declaration_span;
                        known.place.block_span = target.block_span;
                        if !matches!(
                            known.backing.owner.kind,
                            ReferenceBindingKind::Captured { .. }
                        ) {
                            let previous_owner = known.backing.owner.clone();
                            let field = known
                                .place
                                .projections
                                .iter()
                                .rev()
                                .find_map(|projection| match projection {
                                    ReferencePlaceProjection::Field(
                                        ScalarFieldReference::Resolved(field),
                                    ) => Some(field.index),
                                    _ => None,
                                })
                                .expect("typed projected backing field");
                            let owner = ReferenceBindingId {
                                source: target.source.clone(),
                                function_span: target.function_span,
                                declaration_span: target.declaration_span,
                                block_span: target.block_span,
                                kind: ReferenceBindingKind::Projected { field },
                            };
                            transfer_array_family(
                                &mut known.backing,
                                &previous_owner,
                                &owner,
                                false,
                            );
                            known.backing.owner = owner;
                        }
                        state.backing_projected.push(known);
                    }
                    let copied = state
                        .callable_projected
                        .iter()
                        .filter(|known| binding_holder::field_relative(&known.place, source, &field_prefix).is_some())
                        .map(|known| {
                            let mut known = known.clone();
                            known.place = binding_holder::field_relative(&known.place, source, &field_prefix).expect("selected callable-environment prefix");
                            known.place.binding = target.clone();
                            known.place.source = target.source.clone();
                            known.place.function_span = target.function_span;
                            known.place.declaration_span = target.declaration_span;
                            known.place.block_span = target.block_span;
                            known
                        })
                        .collect::<Vec<_>>();
                    if moved {
                        state
                            .callable_projected
                            .retain(|known| binding_holder::field_relative(&known.place, source, &field_prefix).is_none());
                    }
                    state.callable_projected.extend(copied);
                }
                CfgPointKind::ScopeExit => {
                    state
                        .backing_bindings
                        .retain(|binding, _| binding.block_span != point.source_span.range);
                    state
                        .callable_bindings
                        .retain(|binding, _| binding.block_span != point.source_span.range);
                    state
                        .callable_projected
                        .retain(|known| known.place.binding.block_span != point.source_span.range);
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
                CfgPointKind::ReadPlace { .. }
                | CfgPointKind::AssignmentDispose { .. }
                | CfgPointKind::Join
                | CfgPointKind::ValueOutputProjection { .. }
                | CfgPointKind::Evaluate(_)
                | CfgPointKind::Call { .. }
                | CfgPointKind::Bind { .. }
                | CfgPointKind::Assign { .. } => state.value = checked_value.clone(),
                CfgPointKind::AssignThrough { .. } => state.value = checked_value.clone(),
                _ => {}
            }
            if parameter_points.contains(&id) {
                state.backing_value = point.possible_backings.clone();
            }
            let backing_value = match &point.kind {
                _ if control_value => match state.backing_point_values.get(&id) {
                    Some(backings) => backings.clone(),
                    None => Vec::new(),
                },
                CfgPointKind::Call { output, .. } => point.possible_backings.iter().filter(|fact| {
                    matches!(fact.owner.kind, ReferenceBindingKind::CallOutput { point, output: result } if point == id && result == *output)
                }).cloned().collect(),
                CfgPointKind::Join => state.backing_value.clone(),
                _ => point.possible_backings.clone(),
            };
            if backing_value.is_empty() {
                state.backing_point_values.remove(&id);
            } else {
                state.backing_point_values.insert(id, backing_value.clone());
            }
            if matches!(
                point.kind,
                CfgPointKind::Read { .. }
                    | CfgPointKind::Call { .. }
                    | CfgPointKind::Join
                    | CfgPointKind::AssignmentSnapshot { .. }
                    | CfgPointKind::AssignmentResult { .. }
                    | CfgPointKind::ValueOutputProjection { .. }
            ) {
                state.backing_value = backing_value;
            }
            if matches!(point.kind, CfgPointKind::Join | CfgPointKind::Entry) {
                point.possible_backings =
                    state.backing_bindings.values().flatten().cloned().collect();
            }
            save_checked_value(&mut state, point);
            let selected_frozen = match &point.kind {
                CfgPointKind::AssignmentSnapshot {
                    destination_point, ..
                } => state.assignment_destinations[destination_point]
                    .iter()
                    .map(|frozen| selected_source::SelectedSnapshotInput {
                        selection: frozen.selection.clone(),
                        place: frozen.place.clone(),
                    })
                    .collect(),
                _ => Vec::new(),
            };
            if matches!(selected_input, selected_source::SelectedSourceState::Active { .. }) && matches!(point.kind, CfgPointKind::Read { .. } | CfgPointKind::ReadPlace { .. }) {
                if let Err(error) = selected_source::read_choice::original_selectors(&cfg.evaluated_values, &original_read_cfg, &capture_input.policy_context.structs, point) {
                    errors.push(error);
                    continue;
                }
            }
            #[cfg(test)]
            let _snapshot23_call = selected_source::snapshot23_observation::call(
                &cfg.producer_owner, point, &selected_input, &selected_frozen,
            );
            #[cfg(test)]
            if _snapshot23_call.admitted() {
                if let CfgPointKind::AssignmentSnapshot { destination_point, .. } = &point.kind {
                    for (ordinal, frozen) in state.assignment_destinations[destination_point].iter().enumerate() {
                        let access = match &frozen.access {
                            FrozenAssignmentAccess::Direct => 0,
                            FrozenAssignmentAccess::Through { .. } => 1,
                            FrozenAssignmentAccess::Indexed { .. } => 2,
                            FrozenAssignmentAccess::ThroughIndexed { .. } => 3,
                        };
                        selected_source::snapshot23_observation::storage(ordinal, access);
                    }
                }
            }
            let selected_result = selected_source::transfer(
                &mut cfg.evaluated_values,
                &cfg.producer_owner,
                &original_points,
                point,
                &selected_input,
                &state.backing_value,
                &checked_value,
                &cfg.facts,
                &selected_frozen,
            );
            #[cfg(test)]
            selected_source::snapshot23_observation::returned(&selected_result);
            let selected_outputs = match selected_result {
                Ok(output) => {
                    #[cfg(test)]
                    selected_source::snapshot23_observation::receive_ok(&output);
                    output
                }
                Err(error) => {
                    #[cfg(test)]
                    selected_source::snapshot23_observation::receive_error(&error, errors.len());
                    errors.push(error);
                    continue;
                }
            };
            #[cfg(test)]
            drop(_snapshot23_call);
            let paired_outputs = match read_choice::pair(
                point,
                &state,
                selected_outputs,
                &cfg.evaluated_values,
                &last_use,
                element_alternatives.is_empty() && move_alternatives.is_empty() && call_exit_extra_states.is_empty(),
                #[cfg(test)]
                observations,
            ) {
                Ok(outputs) => outputs,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            if let Some(previous) = &mut observed[id.0] {
                for path in &point.backing_argument_paths {
                    if !previous.backing_argument_paths.contains(path) {
                        previous.backing_argument_paths.push(path.clone());
                    }
                }
                for path in &point.global_callable_call_paths {
                    if !previous.global_callable_call_paths.contains(path) {
                        previous.global_callable_call_paths.push(path.clone());
                    }
                }
                for path in &point.call_import_paths {
                    if !previous.call_import_paths.contains(path) {
                        previous.call_import_paths.push(path.clone());
                    }
                }
                for effect in &point.global_callable_output_effects {
                    if !previous.global_callable_output_effects.contains(effect) {
                        previous.global_callable_output_effects.push(effect.clone());
                    }
                }
                for backing in &point.possible_backings {
                    if !previous.possible_backings.contains(backing) {
                        previous.possible_backings.push(backing.clone());
                    }
                }
                for projected in &point.possible_projected_backings {
                    if !previous.possible_projected_backings.contains(projected) {
                        previous.possible_projected_backings.push(projected.clone());
                    }
                }
                previous
                    .possible_environments
                    .extend(point.possible_environments.iter().cloned());
                for projection in &point.possible_projected_environments {
                    if let Some(known) = previous
                        .possible_projected_environments
                        .iter_mut()
                        .find(|known| reference_places_same(&known.place, &projection.place))
                    {
                        known
                            .environments
                            .extend(projection.environments.iter().cloned());
                    } else {
                        previous
                            .possible_projected_environments
                            .push(projection.clone());
                    }
                }
                previous
                    .possible_callee_environments
                    .extend(point.possible_callee_environments.iter().cloned());
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
            for paired_output in paired_outputs {
            let ReferenceWorkItem { mut state, selected: selected_output } = paired_output;
            if matches!(&cfg.points[id.0].kind, CfgPointKind::Call { .. })
                && (!cfg.points[id.0].possible_environments.is_empty()
                    || !cfg.points[id.0].possible_projected_environments.is_empty())
            {
                let mut imported = cfg.points[id.0].possible_environments.clone();
                for projection in &cfg.points[id.0].possible_projected_environments {
                    imported.extend(projection.environments.iter().cloned());
                }
                let module_parameters = match &cfg.points[id.0].kind {
                    CfgPointKind::Call {
                        target:
                            ResolvedCallTarget::ModuleCallable {
                                source,
                                declaration_span,
                                concrete,
                            },
                        ..
                    } => Some(module_callable_parameter_bindings(
                        cfg,
                        source,
                        *declaration_span,
                        concrete,
                    )),
                    _ => None,
                };
                let call_span = cfg.points[id.0].source_span.clone();
                let fresh = cfg.points[id.0].backing_argument_paths.clone();
                forward_imported_environment_backings(
                    cfg,
                    &state,
                    &call_span,
                    &fresh,
                    module_parameters.as_deref(),
                    &imported,
                );
                for relation in &mut state.holder_relations {
                    if imported.contains(&relation.environment) {
                        relation.capture_paths = cfg.environments[relation.environment.index]
                            .capture_paths
                            .clone();
                    }
                }
            }
            if let CfgPointKind::Assign { target, .. } = &original_points[id.0].kind {
                if target.binding.function_span != cfg.points[0].source_span.range
                    && target.binding.kind == ReferenceBindingKind::Declared
                {
                    let effect = GlobalCallableOutputEffectFact {
                        target: target.clone(),
                        before: holder_before.clone(),
                        after: state.holder_relations.clone(),
                        actions: holder_actions.clone(),
                        span: cfg.points[id.0].source_span.clone(),
                    };
                    if let Some(known) = &mut observed[id.0] {
                        if !known.global_callable_output_effects.contains(&effect) {
                            known.global_callable_output_effects.push(effect);
                        }
                    }
                }
            }
            // Collector subunit 5: connected counted-domain entry pairs for
            // received escaping envs are published post-transfer (see
            // append_receipt_counted_pairs); the in-transfer flow stays
            // untouched so visit/round dynamics cannot perturb it.
            if let CfgPointKind::Call {
                target: ResolvedCallTarget::ModuleCallable { .. },
                output: 0,
                ..
            } = &original_points[id.0].kind
            {
                // Cut 3: same-occurrence callee exit effects apply only at
                // their originating caller occurrence and successor edge.
                // The first exit applies in place; further exits fan out as
                // whole-path successor alternatives so distinct branch states
                // remain distinct instead of merging into one summary.
                let target = match &original_points[id.0].kind {
                    CfgPointKind::Call { target, .. } => target.clone(),
                    _ => unreachable!("call occurrence"),
                };
                apply_callee_exit_effects(
                    cfg,
                    &mut state,
                    &mut call_exit_extra_states,
                    id,
                    &target,
                    0,
                    call_exits,
                    &mut errors,
                );
            }
            if let Some(context) = &frozen_phase {
                if let Err(reason) = assignment_frozen_flow::consume(&mut state, context) {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: cfg.points[id.0].source_span.clone(),
                        reason,
                    });
                    continue;
                }
            }
            for successor in &original_points[id.0].successors {
                if matches!(
                    original_points[id.0].kind,
                    CfgPointKind::SelectArrayElement { .. }
                ) && element_alternatives.is_empty()
                {
                    continue;
                }
                if matches!(original_points[id.0].kind, CfgPointKind::Assign { .. }) {
                    state.selected_element = None;
                }
                let edge = CallableHolderEdgeFact {
                    from: id,
                    to: *successor,
                    before: holder_before.clone(),
                    after: state.holder_relations.clone(),
                    actions: holder_actions.clone(),
                };
                if !cfg.holder_edges.contains(&edge) {
                    cfg.holder_edges.push(edge);
                }
                let slot = &mut incoming[successor.0];
                if !element_alternatives.is_empty() {
                    let CfgPointKind::SelectArrayElement { index_point, .. } =
                        &original_points[id.0].kind
                    else {
                        unreachable!()
                    };
                    for selected in &element_alternatives {
                        let mut next = state.clone();
                        next.selected_element = Some((*index_point, selected.clone()));
                        if let Err(error) = conditional_output_production::transport_edge_values(cfg, &mut next, id, *successor) {
                            errors.push(error);
                            continue;
                        }
                        expire_indexed_reference_loans(
                            &mut next,
                            &original_points,
                            &donor_uses,
                            &cfg.facts,
                            id,
                            *successor,
                        );
                        let next = ReferenceWorkItem {
                            state: next,
                            selected: selected_output.clone(),
                        };
                        if !selected_reach[successor.0].contains(&next) {
                            admit_reference_work_item(slot, next);
                        }
                    }
                } else if move_alternatives.is_empty() {
                    let mut next = state.clone();
                    if let Err(error) = conditional_output_production::transport_edge_values(cfg, &mut next, id, *successor) {
                        errors.push(error);
                        continue;
                    }
                    expire_indexed_reference_loans(
                        &mut next,
                        &original_points,
                        &donor_uses,
                        &cfg.facts,
                        id,
                        *successor,
                    );
                    if successor.0 <= id.0 {
                        next.point_values[successor.0..].fill(None);
                        next.callable_point_values[successor.0..].fill(BTreeSet::new());
                        next.callable_point_projected[successor.0..].fill(Vec::new());
                        next.holder_point_values[successor.0..].fill(Vec::new());
                        next.holder_point_sources[successor.0..].fill(Vec::new());
                        next.holder_value.clear();
                        next.holder_sources.clear();
                    }
                    let next = ReferenceWorkItem {
                        state: next,
                        selected: selected_output.clone(),
                    };
                    if !selected_reach[successor.0].contains(&next) {
                        admit_reference_work_item(slot, next);
                    }
                } else {
                    for (place, origin) in &move_alternatives {
                        let mut next = state.clone();
                        if let Err(error) = conditional_output_production::transport_edge_values(cfg, &mut next, id, *successor) {
                            errors.push(error);
                            continue;
                        }
                        expire_indexed_reference_loans(
                            &mut next,
                            &original_points,
                            &donor_uses,
                            &cfg.facts,
                            id,
                            *successor,
                        );
                        if !next.moved.iter().any(|(moved, _)| moved == place) {
                            next.moved.push((place.clone(), origin.clone()));
                        }
                        if successor.0 <= id.0 {
                            next.point_values[successor.0..].fill(None);
                            next.callable_point_values[successor.0..].fill(BTreeSet::new());
                            next.callable_point_projected[successor.0..].fill(Vec::new());
                            next.holder_point_values[successor.0..].fill(Vec::new());
                            next.holder_point_sources[successor.0..].fill(Vec::new());
                            next.holder_value.clear();
                            next.holder_sources.clear();
                        }
                        let next = ReferenceWorkItem {
                            state: next,
                            selected: selected_output.clone(),
                        };
                        if !selected_reach[successor.0].contains(&next) {
                            admit_reference_work_item(slot, next);
                        }
                    }
                }
                if !slot.is_empty() && !queued[successor.0] {
                    queued[successor.0] = true;
                    ready.push_back(*successor);
                }
            }
            // Cut 3: whole-path successor alternatives from further exit
            // facts of the same occurrence; mirrors the plain successor push
            // so each alternative reaches the fixed point independently.
            for successor in &original_points[id.0].successors {
                for alternative in &call_exit_extra_states {
                    let mut next = alternative.clone();
                    if let Err(error) = conditional_output_production::transport_edge_values(cfg, &mut next, id, *successor) {
                        errors.push(error);
                        continue;
                    }
                    expire_indexed_reference_loans(
                        &mut next,
                        &original_points,
                        &donor_uses,
                        &cfg.facts,
                        id,
                        *successor,
                    );
                    if successor.0 <= id.0 {
                        next.point_values[successor.0..].fill(None);
                        next.callable_point_values[successor.0..].fill(BTreeSet::new());
                        next.callable_point_projected[successor.0..].fill(Vec::new());
                        next.holder_point_values[successor.0..].fill(Vec::new());
                        next.holder_point_sources[successor.0..].fill(Vec::new());
                        next.holder_value.clear();
                        next.holder_sources.clear();
                    }
                    let slot = &mut incoming[successor.0];
                    let next = ReferenceWorkItem {
                        state: next,
                        selected: selected_output.clone(),
                    };
                    if !selected_reach[successor.0].contains(&next) {
                        admit_reference_work_item(slot, next);
                    }
                    if !slot.is_empty() && !queued[successor.0] {
                        queued[successor.0] = true;
                        ready.push_back(*successor);
                    }
                }
            }
            if original_points[id.0].successors.is_empty() {
                let exit = CallableHolderExitFact {
                    from: id,
                    before: holder_before.clone(),
                    after: state.holder_relations.clone(),
                    actions: holder_actions.clone(),
                };
                if !cfg.holder_exits.contains(&exit) {
                    cfg.holder_exits.push(exit);
                }
            }
            }
        }
    }
    for pending in pending_captures {
        let built = build_capture_input_path_fact(
            &cfg.producer_owner,
            pending.capture,
            &pending.span,
            &pending.captures,
            &pending.path,
            &pending.provenance,
        );
        match built {
            Ok(fact) => match validate_capture_input_path_fact(&cfg, &fact) {
                Ok(()) => {
                    if !cfg.capture_input_path_facts.contains(&fact) {
                        cfg.capture_input_path_facts.push(fact);
                    }
                }
                Err(error) => errors.push(error),
            },
            Err(error) => errors.push(error),
        }
    }
    for pending in &pending_returns {
        let Some(environment) = cfg.environments.get(pending.environment.index) else {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: pending.return_span.clone(),
                reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
            });
            continue;
        };
        if environment.literal != pending.environment.literal {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: pending.return_span.clone(),
                reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
            });
            continue;
        }
        let fact =
            build_producer_return_case_fact(&cfg.producer_owner, pending, &environment.instance);
        if return_case_selection_refuted(&fact.producer) {
            pending_refuted.push((
                pending.return_point,
                pending.output,
                pending.environment.clone(),
            ));
            continue;
        }
        match validate_producer_return_case_fact(&cfg, &fact) {
            Ok(()) => {
                if !cfg.producer_return_cases.contains(&fact) {
                    cfg.producer_return_cases.push(fact);
                }
            }
            Err(error) => errors.push(error),
        }
    }
    for gap in &pending_return_gaps {
        if return_environment_from_inputs(cfg, &gap.environment) {
            continue;
        }
        if cfg.producer_return_cases.iter().any(|fact| {
            fact.return_point == gap.return_point
                && cfg
                    .points
                    .get(fact.return_point.0)
                    .is_some_and(|point| point.source_span == gap.return_span)
        }) {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: gap.return_span.clone(),
                reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
            });
        }
    }
    for (point, observed) in cfg.points.iter_mut().zip(observed) {
        if let Some(observed) = observed {
            *point = observed;
        }
    }
    for point in &cfg.points {
        let CfgPointKind::ReturnOutput { output, .. } = &point.kind else {
            continue;
        };
        let mut advertised = point.possible_environments.clone();
        for projection in &point.possible_projected_environments {
            advertised.extend(projection.environments.iter().cloned());
        }
        for id in advertised {
            if return_environment_from_inputs(cfg, &id) {
                continue;
            }
            let Some(environment) = cfg.environments.get(id.index) else {
                continue;
            };
            if environment.literal != id.literal {
                continue;
            }
            if cfg.producer_return_cases.iter().any(|fact| {
                fact.table.owner == cfg.producer_owner
                    && fact.return_point == point.id
                    && fact.output == *output
                    && fact.environment == id
                    && fact.alternative.source_index == id.index
                    && fact.alternative.source_instance == environment.instance
            }) {
                continue;
            }
            if pending_refuted
                .iter()
                .any(|(refuted_point, refuted_output, refuted)| {
                    *refuted_point == point.id && *refuted_output == *output && *refuted == id
                })
            {
                continue;
            }
            if !cfg.producer_return_cases.iter().any(|fact| {
                fact.table.owner == cfg.producer_owner
                    && fact.environment == id
                    && fact.alternative.source_index == id.index
                    && fact.alternative.source_instance == environment.instance
                    && (fact.return_point != point.id || fact.output != *output)
            }) {
                continue;
            }
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: point.source_span.clone(),
                reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
            });
        }
    }
    append_receipt_counted_pairs(cfg);
    if let Err(error) = selected_source::local_loan::register(cfg) {
        errors.push(error);
    }
    if let Err(error) = selected_source::validate(cfg) {
        errors.push(error);
    }
    errors.extend(append_install_counted_pairs(cfg));
    append_exit_cell_owner_releases(cfg);
    prune_proven_dropless_drops(cfg);
    let mut unique = Vec::new();
    for error in errors {
        if !unique.contains(&error) {
            unique.push(error);
        }
    }
    unique
}

fn append_receipt_counted_pairs(cfg: &mut ScalarReferenceCfg) {
    // Collector subunit 5: publish one balance-neutral Retain+Drop pair per
    // received escaping environment that owns a connected terminal Drop but
    // matches no counted shape. The pair executes Retain-then-Drop on its
    // edge (net zero on every path it reaches) and confers counted status,
    // so the connected terminal Drop emits exactly once per live generation
    // instead of leaking through the uncounted skip. Pairs attach to the
    // receiving Bind's outgoing edges: imported-holder coverage only
    // accepts counted actions on Bind/CopyCallableFields/Assign/
    // ReturnOutput/ScopeExit points, never on Call points. Envs already
    // counted, or whose Drops retire through cell/array paths, are left
    // untouched.
    let mut pairs: Vec<(
        CfgPointId,
        ReferencePlaceId,
        ReferenceBindingId,
        CallableEnvironmentId,
        SourceSpan,
    )> = Vec::new();
    for point in &cfg.points {
        if !matches!(&point.kind, CfgPointKind::Bind { .. }) {
            continue;
        }
        let CfgPointKind::Bind { binding } = &point.kind else {
            continue;
        };
        for predecessor in output_sequence::binding_source_points(cfg, point.id)
            .into_iter()
            .filter(|candidate| matches!(&candidate.kind, CfgPointKind::Call { .. }))
        {
            for projection in &predecessor.possible_projected_environments {
                if !projection
                    .place
                    .projections
                    .iter()
                    .any(|projection| matches!(projection, ReferencePlaceProjection::Field(_)))
                {
                    continue;
                }
                for environment in &projection.environments {
                    if projection.place.source != environment.literal.source {
                        continue;
                    }
                    if pairs.iter().any(|(known_point, _, _, known, _)| {
                        *known_point == point.id && known == environment
                    }) {
                        continue;
                    }
                    if !receipt_counted_pair_needed(cfg, environment) {
                        continue;
                    }
                    pairs.push((
                        point.id,
                        projection.place.clone(),
                        binding.clone(),
                        environment.clone(),
                        point.source_span.clone(),
                    ));
                }
            }
        }
    }
    for (point, place, binding, environment, span) in pairs {
        let successors = cfg
            .points
            .iter()
            .find(|candidate| candidate.id == point)
            .map(|candidate| candidate.successors.clone())
            .unwrap_or_default();
        if successors.len() != 1 {
            continue;
        }
        // Rebase the call-output field place onto the receiving binding. The
        // CallOutput pseudo-place carries no emitted storage, while the
        // rebased field place resolves through the binding's storage exactly
        // like the field reads that later consume it. The rebased holder is
        // a true post-Bind member, so publish it on the edge relations first
        // (connected_holder_edge requires edge membership).
        let mut target_place = place;
        target_place.source = binding.source.clone();
        target_place.function_span = binding.function_span;
        target_place.declaration_span = binding.declaration_span;
        target_place.block_span = binding.block_span;
        target_place.binding = binding.clone();
        let holder = CallableHolderPlace::Projection(target_place);
        let relation = CallableHolderRelation {
            environment: environment.clone(),
            holders: vec![holder.clone()],
            capture_paths: Vec::new(),
        };
        let retain = CallableHolderAction::Retain {
            environment: environment.clone(),
            from: holder.clone(),
            to: holder.clone(),
            span: span.clone(),
        };
        let drop = CallableHolderAction::Drop {
            environment: environment.clone(),
            holder,
            span,
        };
        for edge in cfg
            .holder_edges
            .iter_mut()
            .filter(|edge| edge.from == point)
        {
            // Never pair onto an edge that already drops the holder: the
            // per-edge drop dedup would swallow the pair's Drop and leave a
            // bare Retain behind.
            if edge.actions.iter().any(|action| {
                matches!(action, CallableHolderAction::Drop { holder: known, .. } if known == &retain_holder(&retain))
            }) {
                continue;
            }
            if !edge
                .after
                .iter()
                .flat_map(|known| known.holders.iter())
                .any(|known| known == &retain_holder(&retain))
            {
                edge.after.push(relation.clone());
            }
            if !edge.actions.contains(&retain) {
                edge.actions.push(retain.clone());
            }
            if !edge.actions.contains(&drop) {
                edge.actions.push(drop.clone());
            }
        }
    }
}

fn retain_holder(action: &CallableHolderAction) -> CallableHolderPlace {
    match action {
        CallableHolderAction::Retain { to, .. } => to.clone(),
        _ => unreachable!("receipt pair retain holder"),
    }
}

fn install_counted_pair_needed(
    cfg: &ScalarReferenceCfg,
    environment: &CallableEnvironmentId,
) -> bool {
    if !receipt_counted_pair_needed(cfg, environment) {
        return false;
    }
    let Some(fact) = cfg
        .environments
        .get(environment.index)
        .filter(|fact| fact.literal == environment.literal)
    else {
        return false;
    };
    fact.backings
        .iter()
        .chain(fact.inputs.iter().flat_map(|input| input.backings.iter()))
        .any(|backing| backing.kind == RuntimeBackingKind::OwnedAllocation)
}

fn append_install_counted_pairs(cfg: &mut ScalarReferenceCfg) -> Vec<ReferenceAnalysisError> {
    // Collector subunit 5 (uncovered envs): an Assign-installed escaping
    // environment owns reachable allocations through its capture inputs but
    // matches no counted shape, so the LLVM skip would retire neither the
    // installed generation nor its cell. Publish one balance-neutral
    // Retain+Drop pair per such environment (net zero on every path it
    // reaches) to confer counted status, mirroring the Bind receipt pairs.
    // Only replaced generations qualify: the installing Assign edge must
    // also retire the previous generation of the same field (a Drop of a
    // different environment on the same holder). Fresh installs keep
    // their existing adopted or counted-cell coverage untouched.
    // The Assign edge itself splits Drops before the store from Retains
    // after it, so the pair cannot live there: a pre-store Drop would
    // retire the overwritten holder, while a Temporary-held Drop would
    // free the generation before its installing Move runs. The pair hosts
    // on the nearest downstream single-successor Bind edge instead, where
    // the installed field holder is live and both actions emit together.
    // Pure-capture installs own no reachable allocations and keep the
    // proven skip; installs with no downstream Bind host are
    // unrepresentable and report B0003 instead of leaking silently.
    let mut installs: Vec<(
        CfgPointId,
        CallableEnvironmentId,
        ReferencePlaceId,
        SourceSpan,
    )> = Vec::new();
    for edge in &cfg.holder_edges {
        let Some(from) = cfg
            .points
            .get(edge.from.0)
            .filter(|point| point.id == edge.from)
        else {
            continue;
        };
        if !matches!(&from.kind, CfgPointKind::Assign { .. }) {
            continue;
        }
        for action in &edge.actions {
            let CallableHolderAction::Move {
                environment,
                from: CallableHolderPlace::Temporary(_),
                to: CallableHolderPlace::Projection(place),
                ..
            } = action
            else {
                continue;
            };
            if !place
                .projections
                .iter()
                .any(|projection| matches!(projection, ReferencePlaceProjection::Field(_)))
            {
                continue;
            }
            if place.source != environment.literal.source {
                continue;
            }
            // Replaced generations only: the same edge must retire the
            // previous generation of the same field holder.
            let replaces = edge.actions.iter().any(|action| {
                matches!(action,
                    CallableHolderAction::Drop {
                        environment: previous,
                        holder: CallableHolderPlace::Projection(previous_place),
                        ..
                    } if previous != environment
                        && previous_place.binding == place.binding
                        && previous_place.projections == place.projections)
            });
            if !replaces {
                continue;
            }
            if installs.iter().any(|(_, known, _, _)| known == environment) {
                continue;
            }
            if !install_counted_pair_needed(cfg, environment) {
                continue;
            }
            installs.push((
                edge.from,
                environment.clone(),
                place.clone(),
                from.source_span.clone(),
            ));
        }
    }
    let mut errors = Vec::new();
    for (assign_point, environment, place, span) in installs {
        // The host must live in the same block as the install: the pair
        // emits during the host binding's emission in program order, right
        // after the install's store, with the installed holder live.
        let host = cfg
            .points
            .iter()
            .filter(|point| {
                point.id.0 > assign_point.0
                    && matches!(&point.kind, CfgPointKind::Bind { binding }
                        if binding.block_span == place.block_span)
                    && point.successors.len() == 1
            })
            .min_by_key(|point| point.id.0);
        let Some(host) = host else {
            // Without any host the install is unrepresentable only when
            // the tail is straight-line; branchy tails keep their current
            // behavior instead of breaking compiling programs.
            let straight = !cfg.points.iter().any(|point| {
                point.id.0 > assign_point.0
                    && (point.successors.len() > 1
                        || matches!(
                            &point.kind,
                            CfgPointKind::Branch { .. }
                                | CfgPointKind::Join
                                | CfgPointKind::LoopTest { .. }
                        ))
            });
            if straight {
                errors.push(ReferenceAnalysisError::MissingContext {
                    source_span: span,
                    reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                });
            }
            continue;
        };
        let host_id = host.id;
        let host_span = host.source_span.clone();
        let [successor] = host.successors.as_slice() else {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: span,
                reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
            });
            continue;
        };
        let successor = *successor;
        let holder = CallableHolderPlace::Projection(place);
        let relation = CallableHolderRelation {
            environment: environment.clone(),
            holders: vec![holder.clone()],
            capture_paths: Vec::new(),
        };
        let retain = CallableHolderAction::Retain {
            environment: environment.clone(),
            from: holder.clone(),
            to: holder.clone(),
            span: host_span.clone(),
        };
        let drop = CallableHolderAction::Drop {
            environment: environment.clone(),
            holder,
            span: host_span,
        };
        let Some(edge) = cfg
            .holder_edges
            .iter_mut()
            .find(|edge| edge.from == host_id && edge.to == successor)
        else {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: span,
                reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
            });
            continue;
        };
        // Never pair onto an edge that already drops the holder: the
        // per-edge drop dedup would swallow the pair's Drop and leave a
        // bare Retain behind.
        if edge.actions.iter().any(|action| {
            matches!(action, CallableHolderAction::Drop { holder: known, .. } if known == &retain_holder(&retain))
        }) {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: span,
                reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
            });
            continue;
        }
        if !edge
            .after
            .iter()
            .flat_map(|known| known.holders.iter())
            .any(|known| known == &retain_holder(&retain))
        {
            edge.after.push(relation);
        }
        if !edge.actions.contains(&retain) {
            edge.actions.push(retain);
        }
        if !edge.actions.contains(&drop) {
            edge.actions.push(drop);
        }
    }
    errors
}

fn append_exit_cell_owner_releases(cfg: &mut ScalarReferenceCfg) {
    // Collector subunit 5 (uncovered envs): an array capture cell carries
    // two LLVM reference counts at function exit — the creator ownership
    // released by the connected DropCaptureCellOwner, and the installing
    // environment's ownership released through that environment's dispose
    // chain. The dispose chain only visits covered, cyclic, or non-array
    // cells, so a local array cell whose environment dies at the same
    // exit would keep its second count (and its backing) forever.
    // Publish the missing second owner release right after the first one
    // on the same terminal exit. Only fires when every guard holds;
    // otherwise the shape keeps its existing coverage:
    // - the captured binding is array-typed (non-array cells already
    //   retire through every environment dispose);
    // - exactly one DropCaptureCellOwner exists for the binding across
    //   all edges and exits (ambiguous owner topologies are untouched);
    // - no literal capturing the binding escapes through a
    //   ReturnTransfer (maker and cyclic cells retire through the maker
    //   exit drop plus the holder-dispose chain);
    // - every capturing environment carries a Drop on the same exit
    //   (the environment dies here, so its cell ownership dies with it;
    //   globally installed generations are excluded).
    let mut owner_counts: BTreeMap<ReferenceBindingId, usize> = BTreeMap::new();
    for action in cfg
        .holder_edges
        .iter()
        .flat_map(|edge| &edge.actions)
        .chain(cfg.holder_exits.iter().flat_map(|exit| &exit.actions))
    {
        if let CallableHolderAction::DropCaptureCellOwner { binding, .. } = action {
            *owner_counts.entry(binding.clone()).or_insert(0) += 1;
        }
    }
    let returned_environments = cfg
        .holder_edges
        .iter()
        .flat_map(|edge| &edge.actions)
        .chain(cfg.holder_exits.iter().flat_map(|exit| &exit.actions))
        .filter_map(|action| match action {
            CallableHolderAction::ReturnTransfer { environment, .. } => Some(environment.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut capturing: BTreeMap<ReferenceBindingId, Vec<(CallableEnvironmentId, ScalarType)>> =
        BTreeMap::new();
    for literal in &cfg.literal_cfgs {
        for capture in &literal.captures {
            let environment = cfg
                .points
                .iter()
                .find(|point| point.id == literal.capture_point)
                .and_then(|point| match &point.kind {
                    CfgPointKind::Capture { environment, .. } => Some(environment.clone()),
                    _ => None,
                });
            let Some(environment) = environment else {
                continue;
            };
            let entry = capturing
                .entry(capture.binding.clone())
                .or_insert_with(Vec::new);
            if !entry.iter().any(|(known, _)| known == &environment) {
                entry.push((environment, capture.ty.clone()));
            }
        }
    }
    for exit in &mut cfg.holder_exits {
        let mut releases: Vec<CallableHolderAction> = Vec::new();
        for (position, action) in exit.actions.iter().enumerate() {
            let CallableHolderAction::DropCaptureCellOwner {
                binding, backing, ..
            } = action
            else {
                continue;
            };
            // Only owned heap backings retire through an emitted release;
            // fixed-value releases never emit, so doubling one would only
            // add noise.
            if !matches!(backing.kind, RuntimeBackingKind::OwnedAllocation) {
                continue;
            }
            if exit.actions[..position].iter().any(|known| known == action)
                || releases.iter().any(|known| known == action)
            {
                continue;
            }
            if owner_counts.get(binding).copied().unwrap_or(0) != 1 {
                continue;
            }
            let Some(capturing) = capturing.get(binding) else {
                continue;
            };
            if capturing.is_empty()
                || !capturing.iter().all(|(_, ty)| {
                    matches!(
                        ty,
                        ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }
                    )
                })
            {
                continue;
            }
            if capturing.iter().any(|(environment, _)| {
                returned_environments
                    .iter()
                    .any(|returned| returned == environment)
            }) {
                continue;
            }
            if !capturing.iter().all(|(environment, _)| {
                exit.actions.iter().any(|known| {
                    matches!(known, CallableHolderAction::Drop { environment: actual, .. } if actual == environment)
                })
            }) {
                continue;
            }
            releases.push(action.clone());
        }
        for release in releases {
            if let Some(position) = exit.actions.iter().position(|known| known == &release) {
                exit.actions.insert(position + 1, release);
            }
        }
    }
}

fn prune_proven_dropless_drops(cfg: &mut ScalarReferenceCfg) {
    // Collector subunit 5 (uncovered envs): a locally created environment
    // with no capture inputs owns no reachable allocations, and its
    // literal allocates no environment at all, so no Drop is needed for
    // it. Recording none is the scalar proof that keeps the LLVM skip
    // legitimate there. Every guard below is load-bearing:
    // - Capture-instance only: call-output environments may carry empty
    //   inputs while owning a callee-allocated generation;
    // - empty inputs and backings: any captured backing may own a live
    //   allocation;
    // - no Retain-family action references the environment anywhere:
    //   a copied holder (for example a copy of a seeded global) keeps
    //   its balancing Drop;
    // - the literal's Capture point lives in this same cfg: imported or
    //   module-level environments are never pruned, even when their
    //   local fact carries no inputs.
    // - the Drop holder is a whole binding on a terminal exit: scope
    //   cleanup of a null environment is vacuous, while Projection-held
    //   Drops on Assign edges mark overwrite releases whose facts other
    //   shapes audit even when the retired generation is pure.
    // Receipt/install pairs only ever target owning environments, so
    // pruning cannot strand a pair's Drop.
    let dropless = cfg
        .environments
        .iter()
        .enumerate()
        .filter(|(_, fact)| {
            matches!(fact.instance, CallableEnvironmentInstance::Capture)
                && fact.inputs.is_empty()
                && fact.backings.is_empty()
        })
        .filter(|(index, fact)| {
            cfg.points.iter().any(|point| {
                matches!(&point.kind, CfgPointKind::Capture { environment, .. }
                    if environment.literal == fact.literal && environment.index == *index)
            })
        })
        .filter(|(index, fact)| {
            let id = CallableEnvironmentId {
                literal: fact.literal.clone(),
                index: *index,
            };
            !cfg.holder_edges
                .iter()
                .flat_map(|edge| &edge.actions)
                .chain(cfg.holder_exits.iter().flat_map(|exit| &exit.actions))
                .any(|action| {
                    matches!(action,
                        CallableHolderAction::Retain { environment, .. }
                        | CallableHolderAction::MoveSelected { environment, .. }
                        | CallableHolderAction::RetainSelected { environment, .. }
                        if environment == &id)
                        || matches!(action,
                            CallableHolderAction::RetainCheckedSelected {
                                old: CallableCellCaseFact::Initialized {
                                    environment_origin,
                                    ..
                                },
                                ..
                            } if environment_origin == &id)
                })
        })
        .map(|(index, fact)| CallableEnvironmentId {
            literal: fact.literal.clone(),
            index,
        })
        .collect::<Vec<_>>();
    if dropless.is_empty() {
        return;
    }
    for exit in &mut cfg.holder_exits {
        exit.actions.retain(|action| {
            !matches!(action,
                CallableHolderAction::Drop {
                    environment,
                    holder: CallableHolderPlace::Binding(_),
                    ..
                } if dropless.contains(environment))
        });
    }
}

pub(super) fn reference_root(place: &ScalarPlace) -> &ScalarPlace {
    match place {
        ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => place,
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => reference_root(base),
    }
}

pub(super) fn collect_reference_projections(
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
    literal_signatures: &'a [closure::ResolvedLiteralSignature],
    sites: Vec<ResolvedCallSite>,
    captures: Vec<(ScalarCaptureOccurrence, CfgPointKind)>,
}

impl ValidatedCallResolver<'_> {
    fn expression(
        &mut self,
        expression: &ScalarExpression,
        names: &BTreeMap<String, ReferenceBindingId>,
        types: &BTreeMap<String, ScalarType>,
    ) -> Result<(), ReferenceAnalysisError> {
        match expression {
            ScalarExpression::FunctionLiteral {
                parameters,
                parameter_spans,
                body,
                span,
            } => {
                let binding_types = types
                    .iter()
                    .filter_map(|(name, ty)| {
                        names.get(name).map(|binding| (binding.clone(), ty.clone()))
                    })
                    .collect();
                let structs = self
                    .modules
                    .iter()
                    .flat_map(|module| module.structs.iter().cloned())
                    .collect::<Vec<_>>();
                let captures =
                    closure::captures(self.source, expression, names, &binding_types, &structs);
                let literal = SourceSpan::new(self.source.clone(), *span);
                let signature = self
                    .literal_signatures
                    .iter()
                    .find(|entry| entry.literal == literal)
                    .ok_or_else(|| ReferenceAnalysisError::MissingContext {
                        source_span: literal.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    })?
                    .signature
                    .clone();
                self.captures.push((
                    ScalarCaptureOccurrence::for_literal(*span, body),
                    CfgPointKind::Capture {
                        literal: *span,
                        environment: CallableEnvironmentId {
                            literal: literal.clone(),
                            index: 0,
                        },
                        signature: signature.clone(),
                        captures,
                        parameters: parameters.clone(),
                        parameter_spans: parameter_spans.clone(),
                        body: body.clone(),
                    },
                ));
                let ScalarType::Callable {
                    parameters: parameter_types,
                    ..
                } = signature
                else {
                    return Err(ReferenceAnalysisError::MissingContext {
                        source_span: literal,
                        reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    });
                };
                if parameters.len() != parameter_spans.len()
                    || parameters.len() != parameter_types.len()
                {
                    return Err(ReferenceAnalysisError::MissingContext {
                        source_span: literal,
                        reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    });
                }
                let mut nested_names = names.clone();
                let mut nested_types = types.clone();
                for ((name, parameter_span), ty) in
                    parameters.iter().zip(parameter_spans).zip(parameter_types)
                {
                    nested_names.insert(
                        name.clone(),
                        ReferenceBindingId {
                            source: self.source.clone(),
                            function_span: *span,
                            declaration_span: *parameter_span,
                            block_span: body.span,
                            kind: ReferenceBindingKind::Declared,
                        },
                    );
                    nested_types.insert(name.clone(), ty);
                }
                let enclosing_span = self.function_span;
                self.function_span = *span;
                let result = self.block(body, &nested_names, &nested_types);
                self.function_span = enclosing_span;
                result?;
            }
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
                        .ok_or_else(|| ReferenceAnalysisError::MissingContext {
                            source_span: SourceSpan::new(self.source.clone(), *span),
                            reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                        })?;
                    let binding_name = receiver.as_ref().ok_or_else(|| {
                        ReferenceAnalysisError::MissingContext {
                            source_span: SourceSpan::new(self.source.clone(), *span),
                            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        }
                    })?;
                    let binding = names.get(binding_name).ok_or_else(|| {
                        ReferenceAnalysisError::MissingContext {
                            source_span: SourceSpan::new(self.source.clone(), *span),
                            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        }
                    })?;
                    let receiver_span =
                        receiver_span.ok_or_else(|| ReferenceAnalysisError::MissingContext {
                            source_span: SourceSpan::new(self.source.clone(), *span),
                            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        })?;
                    Some(ResolvedCallTarget::CallableField {
                        receiver_place: ReferencePlaceId {
                            source: self.source.clone(),
                            function_span: binding.function_span,
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
                    expression: expression.clone(),
                    target,
                    result_type: expression_type_in_module(
                        expression,
                        types,
                        &BTreeSet::new(),
                        &BTreeMap::new(),
                        self.modules
                            .iter()
                            .find(|module| &module.source == self.source)
                            .expect("validated call module"),
                        self.modules,
                        &mut Vec::new(),
                        true,
                    ),
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
                    for (output, target) in assignment.targets.iter().enumerate() {
                        if let ScalarPlace::Name { name, .. } = &target.place {
                            if !names.contains_key(name) {
                                let module = self
                                    .modules
                                    .iter()
                                    .find(|module| module.source == *self.source)
                                    .expect("validated current module");
                                let facts = module.resolved_assignment_types.borrow();
                                let fact = facts
                                    .iter()
                                    .find(|fact| fact.occurrence == *assignment.occurrence)
                                    .ok_or_else(|| ReferenceAnalysisError::MissingContext {
                                        source_span: SourceSpan::new(
                                            self.source.clone(),
                                            target.target_span,
                                        ),
                                        reason:
                                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                    })?;
                                let ty = fact
                                    .replacements
                                    .outputs
                                    .get(output)
                                    .ok_or_else(|| ReferenceAnalysisError::MissingContext {
                                        source_span: SourceSpan::new(
                                            self.source.clone(),
                                            target.target_span,
                                        ),
                                        reason:
                                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                    })?
                                    .ty
                                    .clone();
                                types.insert(name.clone(), ty);
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
) -> Result<
    (
        Vec<ResolvedCallSite>,
        Vec<(ScalarCaptureOccurrence, CfgPointKind)>,
    ),
    ReferenceAnalysisError,
> {
    let current = modules.iter().find(|module| &module.source == source);
    let mut module_callables = BTreeMap::new();
    let mut extern_callables = BTreeMap::new();
    for module in modules {
        let receivers: Vec<Option<String>> = if &module.source == source {
            vec![None]
        } else {
            let aliases: Vec<String> = current
                .iter()
                .flat_map(|current| current.namespace_bindings.iter())
                .filter(|binding| binding.target == module.source)
                .map(|binding| binding.binding.clone())
                .collect();
            if aliases.is_empty() {
                continue;
            }
            aliases.into_iter().map(Some).collect()
        };
        for item in &module.items {
            match item {
                ScalarItem::Function(function) => {
                    for receiver in &receivers {
                        module_callables.insert(
                            (receiver.clone(), function.name.clone()),
                            (module.source.clone(), function.name_span),
                        );
                    }
                }
                ScalarItem::Binding(binding)
                    if matches!(binding.declared_type, ScalarType::Callable { .. }) =>
                {
                    for receiver in &receivers {
                        module_callables.insert(
                            (receiver.clone(), binding.name.clone()),
                            (module.source.clone(), binding.name_span),
                        );
                    }
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
    let literal_signatures = current
        .expect("validated current module")
        .resolved_literals
        .borrow();
    let mut top_level = ValidatedCallResolver {
        source,
        function_span: ByteSpan::new(0, 0),
        module_callables: &module_callables,
        extern_callables: &extern_callables,
        modules,
        literal_signatures: &literal_signatures,
        sites: Vec::new(),
        captures: Vec::new(),
    };
    let mut names = BTreeMap::new();
    let mut types = BTreeMap::new();
    for item in items {
        if let ScalarItem::Function(function) = item {
            types.insert(function.name.clone(), function.signature.clone());
        }
        if let ScalarItem::Binding(binding) = item {
            names.insert(
                binding.name.clone(),
                ReferenceBindingId {
                    source: source.clone(),
                    function_span: ByteSpan::new(0, 0),
                    declaration_span: binding.name_span,
                    block_span: ByteSpan::new(0, 0),
                    kind: ReferenceBindingKind::Declared,
                },
            );
            types.insert(binding.name.clone(), binding.declared_type.clone());
        }
    }
    let mut executable_items = Vec::new();
    for item in items {
        match item {
            ScalarItem::Binding(binding) => top_level.expression(&binding.value, &names, &types)?,
            ScalarItem::Executable(executable) => executable_items.push(executable.clone()),
            _ => {}
        }
    }
    let block = ScalarBlock {
        terminated_items: vec![true; executable_items.len()],
        items: executable_items,
        expressions: Vec::new(),
        final_output_values: Vec::new(),
        span: ByteSpan::new(0, 0),
        occurrence: Box::new(ScalarBlockOccurrence {
            source_span: SourceSpan::new(source.clone(), ByteSpan::new(0, 0)),
            output_projection: Vec::new(),
        }),
        unsafe_context: false,
    };
    top_level.block(&block, &names, &types)?;
    let captures = top_level.captures;
    sites.extend(top_level.sites);
    for item in items {
        if let ScalarItem::Function(function) = item {
            resolve_function_call_sites(
                source,
                function,
                &names,
                &types,
                &module_callables,
                &extern_callables,
                modules,
                &literal_signatures,
                &mut sites,
            )?;
        }
    }
    Ok((sites, captures))
}

fn resolve_function_call_sites(
    source: &SourceIdentity,
    function: &ScalarFunction,
    global_names: &BTreeMap<String, ReferenceBindingId>,
    global_types: &BTreeMap<String, ScalarType>,
    module_callables: &BTreeMap<(Option<String>, String), (SourceIdentity, ByteSpan)>,
    extern_callables: &BTreeMap<(String, String), (SourceIdentity, ByteSpan)>,
    modules: &[ScalarModule],
    literal_signatures: &[closure::ResolvedLiteralSignature],
    sites: &mut Vec<ResolvedCallSite>,
) -> Result<(), ReferenceAnalysisError> {
    for arm in &function.overload_arms {
        resolve_function_call_sites(
            source,
            arm,
            global_names,
            global_types,
            module_callables,
            extern_callables,
            modules,
            literal_signatures,
            sites,
        )?;
    }
    let mut resolver = ValidatedCallResolver {
        source,
        function_span: function.span,
        module_callables,
        extern_callables,
        modules,
        literal_signatures,
        sites: Vec::new(),
        captures: Vec::new(),
    };
    let mut parameter_names = global_names.clone();
    let mut parameter_types = global_types.clone();
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
    top_level_cfg: &mut ScalarReferenceCfg,
    call_sites: &[ResolvedCallSite],
    top_level_captures: &[(ScalarCaptureOccurrence, CfgPointKind)],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
    literal_signatures: &[closure::ResolvedLiteralSignature],
    value_types: &assignment::SourceValueTypeFacts,
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
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
    let global_bindings = items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) => Some((
                binding.name.clone(),
                (
                    ReferenceBindingId {
                        source: source.clone(),
                        function_span: ByteSpan::new(0, 0),
                        declaration_span: binding.name_span,
                        block_span: ByteSpan::new(0, 0),
                        kind: ReferenceBindingKind::Declared,
                    },
                    binding.declared_type.clone(),
                ),
            )),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let name_globals = name_resolution::global_declarations(source, items);
    let mut errors = Vec::new();
    errors.extend(record_module_reference_inputs(
        source,
        items,
        top_level_cfg,
        call_sites,
        top_level_captures,
        structs,
        enums,
        assignment_outputs,
        literal_signatures,
        &module_names,
        value_types,
        #[cfg(test)]
        trace,
    ));
    let name_originals = match name_resolution::canonical_module_root(top_level_cfg, source, items) {
        Ok(root) => vec![root],
        Err(error) => { errors.push(error); return errors; },
    };
    for item in items.iter_mut() {
        if let ScalarItem::Function(function) = item {
            errors.extend(record_function_reference_origins_worker(
                source,
                function,
                &module_names,
                &global_bindings,
                &name_globals,
                &name_originals,
                call_sites,
                structs,
                enums,
                assignment_outputs,
                literal_signatures,
                value_types,
                #[cfg(test)]
                trace,
            ));
        }
    }
    if errors.is_empty() {
        let mut staged = top_level_cfg.clone();
        staged.source_capture_inventory.phase = SourceCaptureConstructionPhase::Complete;
        let checked = source_boundary::module_preflight_forest(source, items, &staged, structs, enums, SourceCaptureConstructionPhase::Complete)
            .and_then(|forest| source_boundary::preflight_input(&staged.producer_owner, &forest, BTreeMap::new()))
            .and_then(|input| source_boundary::validate_capture_preflight(&staged, &input));
        match checked { Ok(()) => top_level_cfg.source_capture_inventory.phase = SourceCaptureConstructionPhase::Complete, Err(error) => errors.push(error) }
    }
    errors
}

fn module_backing_point(
    builder: &mut ReferenceFlowBuilder,
    target: &ReferenceBindingId,
    ty: &ScalarType,
    value: &ScalarExpression,
    rhs_point: Option<CfgPointId>,
    allocation: bool,
    span: ByteSpan,
    scope: ReferenceScopeId,
) -> Result<(), ReferenceAnalysisError> {
    let transfer = if allocation {
        RuntimeBackingTransfer::Allocate
    } else if let ScalarExpression::Name { name, span } = value {
        let source = builder.binding(name, *span)?;
        let source_ty = builder.binding_types.get(&source).ok_or_else(|| {
            builder.missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
        })?;
        match copy_policy(
            source_ty,
            &builder.structs,
            &builder.enums,
            &builder.generic_parameters,
        )
        .map_err(|_| {
            builder.missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
        })? {
            CopyPolicy::Copy => RuntimeBackingTransfer::CopyFixed { source },
            CopyPolicy::Move => RuntimeBackingTransfer::Move { source },
            CopyPolicy::Deferred => {
                return Err(builder
                    .missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding))
            }
        }
    } else if let Some(point) = rhs_point {
        if matches!(ty, ScalarType::Array { .. }) {
            RuntimeBackingTransfer::CreateFixed
        } else if let CfgPointKind::Call { output, .. } = builder.cfg.points[point.0].kind {
            RuntimeBackingTransfer::Move {
                source: ReferenceBindingId {
                    source: builder.source.clone(),
                    function_span: builder.function_span,
                    declaration_span: builder.cfg.points[point.0].source_span.range,
                    block_span: builder.function_span,
                    kind: ReferenceBindingKind::CallOutput { point, output },
                },
            }
        } else if matches!(ty, ScalarType::Array { .. }) {
            RuntimeBackingTransfer::CreateFixed
        } else {
            return Err(
                builder.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
            );
        }
    } else if matches!(ty, ScalarType::Array { .. }) {
        RuntimeBackingTransfer::CreateFixed
    } else {
        return Err(
            builder.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
        );
    };
    builder.point(
        CfgPointKind::Backing {
            target: target.clone(),
            transfer,
            materialize: matches!(ty, ScalarType::RuntimeArray { .. }),
        },
        span,
        scope,
        BTreeSet::new(),
    );
    Ok(())
}

fn module_backing_place(
    builder: &mut ReferenceFlowBuilder,
    target: ReferencePlaceId,
    value: &ScalarExpression,
    rhs_point: Option<CfgPointId>,
    span: ByteSpan,
    scope: ReferenceScopeId,
) -> Result<(), ReferenceAnalysisError> {
    let transfer = match value {
        ScalarExpression::Name { name, span } => {
            let source = builder.binding(name, *span)?;
            let source_ty = builder.binding_types.get(&source).ok_or_else(|| {
                builder.missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
            })?;
            match copy_policy(
                source_ty,
                &builder.structs,
                &builder.enums,
                &builder.generic_parameters,
            )
            .map_err(|_| {
                builder.missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
            })? {
                CopyPolicy::Copy => RuntimeBackingTransfer::CopyFixed { source },
                CopyPolicy::Move => RuntimeBackingTransfer::Move { source },
                CopyPolicy::Deferred => {
                    return Err(builder.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ))
                }
            }
        }
        ScalarExpression::Call { .. } => {
            let point = rhs_point.ok_or_else(|| {
                builder.missing_context(
                    span,
                    ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                )
            })?;
            let CfgPointKind::Call { output, .. } = &builder.cfg.points[point.0].kind else {
                return Err(builder.missing_context(
                    span,
                    ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                ));
            };
            RuntimeBackingTransfer::Move {
                source: ReferenceBindingId {
                    source: builder.source.clone(),
                    function_span: builder.function_span,
                    declaration_span: builder.cfg.points[point.0].source_span.range,
                    block_span: builder.function_span,
                    kind: ReferenceBindingKind::CallOutput {
                        point,
                        output: *output,
                    },
                },
            }
        }
        _ => {
            return Err(
                builder.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
            )
        }
    };
    builder.point(
        CfgPointKind::BackingPlace { target, transfer },
        span,
        scope,
        BTreeSet::new(),
    );
    Ok(())
}

fn record_nested_top_level_place_calls(
    builder: &mut ReferenceFlowBuilder,
    place: &ScalarPlace,
    scope: ReferenceScopeId,
) -> Result<(), ReferenceAnalysisError> {
    match place {
        ScalarPlace::Name { .. } => Ok(()),
        ScalarPlace::Dereference { pointer, .. } => {
            record_nested_top_level_calls(builder, pointer, scope)
        }
        ScalarPlace::Field { base, .. } => {
            record_nested_top_level_place_calls(builder, base, scope)
        }
        ScalarPlace::Index { base, index, .. } => {
            record_nested_top_level_place_calls(builder, base, scope)?;
            record_nested_top_level_calls(builder, index, scope)
        }
    }
}

fn record_nested_top_level_calls(
    builder: &mut ReferenceFlowBuilder,
    expression: &ScalarExpression,
    scope: ReferenceScopeId,
) -> Result<(), ReferenceAnalysisError> {
    match expression {
        ScalarExpression::Assignment(assignment) => {
            if !builder
                .cfg
                .evaluated_values
                .assignment_evaluations
                .iter()
                .any(|evaluation| evaluation.operation.occurrence == *assignment.occurrence)
            {
                builder.expression(expression, scope)?;
            }
        }
        ScalarExpression::Call { occurrence, .. } => {
            if !builder
                .cfg
                .call_points
                .iter()
                .any(|call| call.occurrence == **occurrence)
            {
                builder.expression(expression, scope)?;
            }
        }
        ScalarExpression::Dereference { place, .. }
        | ScalarExpression::IndexedRead { place, .. }
        | ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. } => {
            record_nested_top_level_place_calls(builder, place, scope)?;
        }
        ScalarExpression::Binary { left, right, .. } => {
            record_nested_top_level_calls(builder, left, scope)?;
            record_nested_top_level_calls(builder, right, scope)?;
        }
        ScalarExpression::Unary { operand, .. } => {
            record_nested_top_level_calls(builder, operand, scope)?;
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                record_nested_top_level_calls(builder, element, scope)?;
            }
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                record_nested_top_level_calls(builder, &field.value, scope)?;
            }
        }
        ScalarExpression::FunctionLiteral { .. }
        | ScalarExpression::If { .. }
        | ScalarExpression::UnitIf { .. }
        | ScalarExpression::Block(_)
        | ScalarExpression::Name { .. }
        | ScalarExpression::Member { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. } => {}
    }
    Ok(())
}

fn record_top_level_item_points(cfg: &mut ScalarReferenceCfg, item: &ScalarItem, start: usize) {
    let occurrence = match item {
        ScalarItem::Binding(binding)
        | ScalarItem::Executable(ScalarBlockItem::LocalBinding(binding)) => {
            ScalarTopLevelOccurrence::Binding(binding.occurrence.as_ref().clone())
        }
        ScalarItem::Executable(ScalarBlockItem::Assignment(assignment)) => {
            ScalarTopLevelOccurrence::Assignment(assignment.occurrence.as_ref().clone())
        }
        ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Call {
            occurrence,
            ..
        })) => ScalarTopLevelOccurrence::Call(occurrence.as_ref().clone()),
        ScalarItem::Executable(ScalarBlockItem::Expression(expression)) => {
            ScalarTopLevelOccurrence::Expression(SourceSpan::new(
                cfg.points[0].source_span.source.clone(),
                expression_span(expression),
            ))
        }
        ScalarItem::Executable(ScalarBlockItem::While(_)) => {
            assert_eq!(
                cfg.points.len(),
                start,
                "unrepresented top-level emission occurrence"
            );
            return;
        }
        ScalarItem::Function(_) | ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {
            unreachable!("non-executable top-level item")
        }
    };
    for index in start..cfg.points.len() {
        let point = CfgPointId(index);
        let call = cfg.call_points.iter().find(|call| {
            call.evaluated_point == point
                || call
                    .projections
                    .iter()
                    .any(|projection| projection.point == point)
        });
        let occurrence = match call {
            Some(call) => ScalarTopLevelOccurrence::Call(call.occurrence.clone()),
            None => {
                let binding = cfg.binding_points.iter().find(|event| event.point == point);
                match binding {
                    Some(binding) => ScalarTopLevelOccurrence::Binding(binding.occurrence.clone()),
                    None => occurrence.clone(),
                }
            }
        };
        cfg.top_level.points.push(ScalarTopLevelPoint {
            point,
            execution: ScalarTopLevelPointExecution::Connected {
                occurrence,
                outgoing: Vec::new(),
            },
        });
    }
    if let ScalarItem::Binding(binding)
    | ScalarItem::Executable(ScalarBlockItem::LocalBinding(binding)) = item
    {
        record_top_level_binding(cfg, binding);
    }
}

fn record_top_level_binding(cfg: &mut ScalarReferenceCfg, binding: &ScalarBinding) {
    for (output, receiver) in binding.receivers.iter().enumerate() {
        let points = cfg
            .binding_points
            .iter()
            .filter(|event| event.occurrence == *binding.occurrence && event.output == output)
            .map(|event| event.point)
            .collect::<Vec<_>>();
        let execution = if points.is_empty() {
            if matches!(
                receiver.ty,
                ScalarType::Unit
                    | ScalarType::Bool
                    | ScalarType::I8
                    | ScalarType::I16
                    | ScalarType::I32
                    | ScalarType::I64
                    | ScalarType::I128
                    | ScalarType::U8
                    | ScalarType::U16
                    | ScalarType::U32
                    | ScalarType::U64
                    | ScalarType::U128
                    | ScalarType::F32
                    | ScalarType::F64
                    | ScalarType::Char
                    | ScalarType::ArtifactId
                    | ScalarType::RawPointer(_)
            ) {
                ScalarTopLevelBindingExecution::EmittedWithoutHolderAction
            } else {
                ScalarTopLevelBindingExecution::UnrepresentedHolderAction
            }
        } else {
            ScalarTopLevelBindingExecution::Connected { points }
        };
        cfg.top_level.bindings.push(ScalarTopLevelBinding {
            occurrence: binding.occurrence.as_ref().clone(),
            output,
            execution,
        });
    }
}

fn record_top_level_block_bindings(cfg: &mut ScalarReferenceCfg, block: &ScalarBlock) {
    for item in &block.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => record_top_level_binding(cfg, binding),
            ScalarBlockItem::Expression(ScalarExpression::Block(inner)) => {
                record_top_level_block_bindings(cfg, inner)
            }
            ScalarBlockItem::While(loop_) => record_top_level_block_bindings(cfg, &loop_.body),
            ScalarBlockItem::Expression(_) | ScalarBlockItem::Assignment(_) => {}
        }
    }
}

fn record_top_level_assignment(
    cfg: &mut ScalarReferenceCfg,
    assignment: &ScalarAssignment,
    names: &BTreeMap<String, ReferenceBindingId>,
    binding_types: &HashMap<ReferenceBindingId, ScalarType>,
    structs: &[ScalarStruct],
) {
    let points = cfg
        .assignment_points
        .iter()
        .filter(|event| event.occurrence == *assignment.occurrence)
        .map(|event| event.point)
        .collect::<Vec<_>>();
    let fully_represented =
        assignment
            .targets
            .iter()
            .enumerate()
            .all(|(position, target)| {
                if cfg.assignment_points.iter().any(|event| {
                    event.occurrence == *assignment.occurrence && event.target == position
                }) {
                    return true;
                }
                module_assignment_place_type(&target.place, names, binding_types, structs)
                    .is_some_and(|ty| {
                        !matches!(
                            ty,
                            ScalarType::CheckedReference { .. }
                                | ScalarType::Callable { .. }
                                | ScalarType::Struct(_)
                                | ScalarType::RuntimeArray { .. }
                                | ScalarType::Array { .. }
                        )
                    })
            });
    let execution = if !fully_represented {
        ScalarTopLevelAssignmentExecution::UnrepresentedHolderAction
    } else if points.is_empty() {
        ScalarTopLevelAssignmentExecution::EmittedWithoutHolderAction
    } else {
        ScalarTopLevelAssignmentExecution::Connected { points }
    };
    cfg.top_level.assignments.push(ScalarTopLevelAssignment {
        occurrence: assignment.occurrence.as_ref().clone(),
        execution,
    });
}

fn module_assignment_place_type(
    place: &ScalarPlace,
    names: &BTreeMap<String, ReferenceBindingId>,
    binding_types: &HashMap<ReferenceBindingId, ScalarType>,
    structs: &[ScalarStruct],
) -> Option<ScalarType> {
    match place {
        ScalarPlace::Name { name, .. } => names
            .get(name)
            .and_then(|binding| binding_types.get(binding))
            .cloned(),
        ScalarPlace::Field {
            field: ScalarFieldReference::Resolved(field),
            ..
        } => structs
            .iter()
            .find(|structure| structure.id == field.structure)
            .and_then(|structure| {
                structure
                    .fields
                    .iter()
                    .find(|declared| declared.id == *field)
            })
            .map(|declared| declared.ty.clone()),
        ScalarPlace::Field {
            field: ScalarFieldReference::Unresolved { .. },
            ..
        } => None,
        ScalarPlace::Index { base, .. } => {
            match module_assignment_place_type(base, names, binding_types, structs)? {
                ScalarType::RuntimeArray { element, .. } | ScalarType::Array { element, .. } => {
                    Some(*element)
                }
                _ => None,
            }
        }
        ScalarPlace::Dereference { pointer, .. } => match pointer.as_ref() {
            ScalarExpression::Name { name, .. } => match binding_types.get(names.get(name)?)? {
                ScalarType::CheckedReference { inner, .. } | ScalarType::RawPointer(inner) => {
                    Some(*inner.clone())
                }
                _ => None,
            },
            _ => None,
        },
    }
}

fn module_binding_captures(
    builder: &mut ReferenceFlowBuilder,
    item_span: ByteSpan,
    captures: &[(ScalarCaptureOccurrence, CfgPointKind)],
    placed: &mut BTreeSet<usize>,
    scope: ReferenceScopeId,
) -> Result<(), ReferenceAnalysisError> {
    for (index, (occurrence, kind)) in captures.iter().enumerate() {
        let span = &occurrence.source_span;
        if !placed.contains(&index) && item_span.start <= span.range.start
            && span.range.end <= item_span.end
            && !captures.iter().any(|(parent, _)| {
                let parent = &parent.source_span;
                parent != span
                    && parent.range.start <= span.range.start
                    && span.range.end <= parent.range.end
            })
        {
            if !builder.cfg.capture_points.iter().any(|capture| capture.occurrence == *occurrence
                && capture.role == ScalarCapturePointRole::Capture) {
                module_capture_point(builder, occurrence, kind, scope)?;
            }
            placed.insert(index);
        }
    }
    Ok(())
}

fn record_module_reference_inputs(
    source: &SourceIdentity,
    items: &[ScalarItem],
    top_level_cfg: &mut ScalarReferenceCfg,
    call_sites: &[ResolvedCallSite],
    top_level_captures: &[(ScalarCaptureOccurrence, CfgPointKind)],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
    literal_signatures: &[closure::ResolvedLiteralSignature],
    module_names: &HashSet<String>,
    value_types: &assignment::SourceValueTypeFacts,
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    let module_span = ByteSpan::new(0, 0);
    let scope = ReferenceScopeId(0);
    let mut builder = ReferenceFlowBuilder {
        rebind_association_count: 0,
        rebind_associations: BTreeMap::new(),
        source: source.clone(),
        function_span: module_span,
        assignment_context_span: module_span,
        names: BTreeMap::new(),
        global_bindings: BTreeMap::new(),
        module_names: module_names.clone(),
        call_sites: call_sites.to_vec(),
        origins: HashMap::new(),
        checked: HashSet::new(),
        binding_types: HashMap::new(),
        structs: structs.to_vec(),
        enums: enums.to_vec(),
        generic_scope: ProducerCfgOwnerId::Module(source.clone()),
        generic_declarations: Vec::new(),
        generic_parameters: BTreeSet::new(),
        assignment_outputs: assignment_outputs.clone(),
        value_types: value_types.clone(),
        literal_signatures: literal_signatures.to_vec(),
        dereference_points: HashMap::new(),
        index_points: HashMap::new(),
        array_elements: Box::new(HashMap::new()),
        cfg: ScalarReferenceCfg::new(ProducerCfgOwnerId::Module(source.clone())),
        next_scope: 0,
        cursor: None,
    };
    if source_boundary::module_resolver_input::validate_endpoint(top_level_cfg).is_err() {
        return vec![ReferenceAnalysisError::MissingContext { source_span: SourceSpan::new(source.clone(), ByteSpan::new(0, 0)), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence }];
    }
    builder.cfg.module_resolver_input = top_level_cfg.module_resolver_input.clone();
    builder.cfg.name_resolution_context = name_resolution::module_context(source, items, &builder.cfg.top_level, &builder.value_types.expressions);
    builder.point(CfgPointKind::Entry, module_span, scope, BTreeSet::new());
    builder.cfg.top_level.points.push(ScalarTopLevelPoint {
        point: CfgPointId(0),
        execution: ScalarTopLevelPointExecution::Connected {
            occurrence: ScalarTopLevelOccurrence::Entry(source.clone()),
            outgoing: Vec::new(),
        },
    });
    let mut placed_captures = BTreeSet::new();
    let mut errors = Vec::new();
    for item in items {
        let item_span = match item {
            ScalarItem::Binding(binding) => binding.span,
            ScalarItem::Executable(ScalarBlockItem::Assignment(assignment)) => assignment.span,
            ScalarItem::Executable(ScalarBlockItem::Expression(expression)) => {
                expression_span(expression)
            }
            ScalarItem::Executable(ScalarBlockItem::While(loop_item)) => loop_item.span,
            ScalarItem::Executable(ScalarBlockItem::LocalBinding(binding)) => binding.span,
            ScalarItem::Function(_) | ScalarItem::Namespace(_) | ScalarItem::Extern(_) => continue,
        };
        let start = builder.cfg.points.len();
        let nested = match item {
            ScalarItem::Binding(binding)
                if binding.receivers[0].allocation_length.is_none()
                    && !binding.receivers.iter().any(|receiver| {
                        matches!(
                            receiver.ty,
                            ScalarType::RuntimeArray { .. } | ScalarType::Array { .. }
                        )
                    }) =>
            {
                Some(&binding.value)
            }
            ScalarItem::Executable(ScalarBlockItem::Expression(expression)) => Some(expression),
            _ => None,
        };
        if let Some(
            expression @ (ScalarExpression::Dereference { .. }
            | ScalarExpression::IndexedRead { .. }
            | ScalarExpression::RawAddress { .. }
            | ScalarExpression::CheckedAddress { .. }),
        ) = nested
        {
            if let Err(error) = record_nested_top_level_calls(&mut builder, expression, scope) {
                errors.push(error);
            }
        }
        if let ScalarItem::Binding(binding) = item {
            let id = ReferenceBindingId {
                source: source.clone(),
                function_span: module_span,
                declaration_span: binding.name_span,
                block_span: module_span,
                kind: ReferenceBindingKind::Declared,
            };
            let runtime_value = matches!(
                binding.declared_type,
                ScalarType::RuntimeArray { .. } | ScalarType::Array { .. }
            );
            let runtime_outputs = binding.receivers.iter().any(|receiver| {
                matches!(
                    receiver.ty,
                    ScalarType::RuntimeArray { .. } | ScalarType::Array { .. }
                )
            });
            let value = if matches!(binding.value, ScalarExpression::Block(_) | ScalarExpression::If { .. }) {
                match builder.expression(&binding.value, scope) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        errors.push(error);
                        None
                    }
                }
            } else if matches!(binding.declared_type, ScalarType::CheckedReference { .. }) {
                match &binding.value {
                    ScalarExpression::CheckedAddress { .. }
                    | ScalarExpression::Name { .. }
                    | ScalarExpression::Call { .. } => {
                        match builder.expression(&binding.value, scope) {
                            Ok(value) => Some(value),
                            Err(error) => {
                                errors.push(error);
                                None
                            }
                        }
                    }
                    _ => Some(ReferenceValue::Uncomputed),
                }
            } else if binding.receivers[0].allocation_length.is_some()
                || matches!(
                    binding.value,
                    ScalarExpression::Call { .. }
                        | ScalarExpression::Binary { .. }
                        | ScalarExpression::Unary { .. }
                )
                || runtime_outputs
                || (matches!(&binding.declared_type, ScalarType::Array { element, .. } if matches!(element.as_ref(), ScalarType::Callable { .. }))
                    && matches!(binding.value, ScalarExpression::ArrayLiteral { .. }))
                || (matches!(
                    binding.declared_type,
                    ScalarType::Struct(_) | ScalarType::Callable { .. }
                ) && matches!(
                    binding.value,
                    ScalarExpression::Call { .. } | ScalarExpression::Name { .. }
                ))
            {
                let expression = match &binding.receivers[0].allocation_length {
                    Some(length) => length,
                    None => &binding.value,
                };
                match builder.expression(expression, scope) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        errors.push(error);
                        None
                    }
                }
            } else {
                match builder.expression(&binding.value, scope) {
                    Ok(value) => Some(value),
                    Err(error) => { errors.push(error); None }
                }
            };
            let array_value_point = if matches!(&binding.declared_type, ScalarType::Array { .. })
                && matches!(binding.value, ScalarExpression::ArrayLiteral { .. })
            {
                match builder.expression_value_point(&binding.value, 0) {
                    Ok(point) => Some(point),
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                }
            } else {
                None
            };
            let rhs_point = if (value.is_some() && !binding.is_allocation) || runtime_outputs
                || matches!(binding.value, ScalarExpression::If { .. } | ScalarExpression::Block(_))
                || (matches!(binding.declared_type, ScalarType::CheckedReference { .. })
                    && matches!(
                        binding.value,
                        ScalarExpression::CheckedAddress { .. }
                            | ScalarExpression::Name { .. }
                            | ScalarExpression::Call { .. }
                            | ScalarExpression::Block(_)
                    ))
                || matches!(
                    (&binding.declared_type, &binding.value),
                    (
                        ScalarType::CheckedReference { .. },
                        ScalarExpression::Call { .. }
                    )
                )
                || (matches!(
                    binding.declared_type,
                    ScalarType::Struct(_) | ScalarType::Callable { .. }
                ) && matches!(
                    binding.value,
                    ScalarExpression::Call { .. } | ScalarExpression::Name { .. }
                ))
                || (binding.receivers.len() > 1
                    && matches!(binding.value, ScalarExpression::Call { .. }))
            {
                match builder.expression_value_point(&binding.value, 0) {
                    Ok(point) => Some(point),
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                }
            } else {
                None
            };
            let length_point = if let Some(length) = &binding.receivers[0].allocation_length {
                match builder.expression_value_point(length, 0) {
                    Ok(point) => Some(point),
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                }
            } else {
                builder.cursor
            };
            if runtime_value
                || matches!(
                    binding.declared_type,
                    ScalarType::CheckedReference {
                        mutability: ScalarReferenceMutability::Mutable,
                        ..
                    }
                )
            {
                if let Err(error) = builder.move_value(&binding.value, scope) {
                    errors.push(error);
                }
            }
            builder.names.insert(binding.name.clone(), id.clone());
            builder
                .binding_types
                .insert(id.clone(), binding.declared_type.clone());
            if let Err(error) = builder.register_source_binding(&id, &binding.declared_type) {
                errors.push(error);
            }
            if matches!(binding.declared_type, ScalarType::CheckedReference { .. }) {
                builder.checked_storage(
                    &id,
                    ScalarPlace::Name {
                        name: binding.name.clone(),
                        span: binding.name_span,
                    },
                );
            }
            if let Err(error) = builder.record_backing_length_site(binding, &id, length_point) {
                errors.push(error);
            }
            if matches!(binding.declared_type, ScalarType::CheckedReference { .. }) {
                builder.checked.insert(id.clone());
            }
            if binding.is_allocation {
                builder.cfg.allocations.insert(id.clone());
            }
            if !matches!(binding.value, ScalarExpression::StructLiteral { .. })
                && array_value_point.is_none()
            {
                if let Err(error) = module_binding_captures(
                    &mut builder,
                    item_span,
                    top_level_captures,
                    &mut placed_captures,
                    scope,
                ) { errors.push(error); }
            }
            match value {
                Some(ReferenceValue::Checked(origins)) => {
                    builder.origins.insert(id.clone(), origins.clone());
                    builder.point(
                        CfgPointKind::Bind {
                            binding: id.clone(),
                        },
                        binding.name_span,
                        scope,
                        origins,
                    );
                }
                Some(ReferenceValue::Uncomputed | ReferenceValue::Other)
                    if runtime_outputs
                        || matches!(binding.value, ScalarExpression::Call { .. } | ScalarExpression::If { .. } | ScalarExpression::Block(_)) =>
                {
                    builder.point(
                        CfgPointKind::Bind {
                            binding: id.clone(),
                        },
                        binding.name_span,
                        scope,
                        BTreeSet::new(),
                    );
                }
                Some(ReferenceValue::Uncomputed | ReferenceValue::Other)
                    if matches!(binding.declared_type, ScalarType::CheckedReference { .. })
                        && rhs_point.is_some() =>
                {
                    builder.uncomputed_point(
                        CfgPointKind::Bind {
                            binding: id.clone(),
                        },
                        binding.name_span,
                        scope,
                    );
                }
                Some(ReferenceValue::Uncomputed | ReferenceValue::Other)
                    if matches!(
                        binding.declared_type,
                        ScalarType::Struct(_) | ScalarType::Callable { .. }
                    ) =>
                {
                    builder.point(
                        CfgPointKind::Bind {
                            binding: id.clone(),
                        },
                        binding.name_span,
                        scope,
                        BTreeSet::new(),
                    );
                }
                Some(ReferenceValue::Uncomputed | ReferenceValue::Other) => {
                    builder.point(
                        CfgPointKind::Bind { binding: id.clone() },
                        binding.name_span,
                        scope,
                        BTreeSet::new(),
                    );
                }
                None => {
                    builder.point(
                        CfgPointKind::Bind {
                            binding: id.clone(),
                        },
                        binding.name_span,
                        scope,
                        BTreeSet::new(),
                    );
                }
            }
            if let Some(point) = builder.cursor {
                if matches!(&builder.cfg.points[point.0].kind, CfgPointKind::Bind { binding: recorded } if recorded == &id)
                {
                    if matches!(binding.declared_type, ScalarType::CheckedReference { .. }) {
                        if let Some(value_point) = rhs_point {
                            builder.checked_bind_input(point, value_point);
                        }
                    }
                    if let Err(error) = builder.binding_source_point(binding, 0, point) {
                        errors.push(error);
                    }
                }
            }
            if let Some(value_point) = array_value_point {
                if let Err(error) =
                    builder.bind_callable_array_elements(binding, value_point, scope)
                {
                    errors.push(error);
                }
            }
            if matches!(binding.value, ScalarExpression::StructLiteral { .. }) {
                if let Err(error) = module_binding_captures(
                    &mut builder,
                    item_span,
                    top_level_captures,
                    &mut placed_captures,
                    scope,
                ) { errors.push(error); }
            }
            if let Err(error) = builder.copy_evaluated_aggregate_binding(binding, &id, 0, scope) { errors.push(error); }
            if let (ScalarType::Struct(structure), ScalarExpression::StructLiteral { fields, .. }) =
                (&binding.declared_type, &binding.value)
            {
                let literal = match builder.module_literal_members(binding, &id) {
                    Ok(literal) => literal,
                    Err(error) => { errors.push(error); continue; }
                };
                for (field, member) in fields.iter().zip(&literal.members) {
                    let declared = structs
                        .iter()
                        .find(|item| item.id == *structure)
                        .and_then(|item| item.fields.iter().find(|item| item.name == field.name));
                    let Some(declared) = declared else {
                        errors.push(builder.missing_context(
                            field.name_span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        ));
                        continue;
                    };
                    if matches!(declared.ty, ScalarType::RuntimeArray { .. }) {
                        let place = ScalarPlace::Field {
                            base: Box::new(ScalarPlace::Name {
                                name: binding.name.clone(),
                                span: binding.name_span,
                            }),
                            field: ScalarFieldReference::Resolved(declared.id.clone()),
                            span: field.span,
                        };
                        match builder
                            .assignment_place(&place, field.span)
                            .and_then(|target| {
                                module_backing_place(
                                    &mut builder,
                                    target,
                                    &field.value,
                                    Some(member.value.point),
                                    field.span,
                                    scope,
                                )
                            }) {
                            Ok(()) => {}
                            Err(error) => errors.push(error),
                        }
                    }
                    if contains_projected_runtime_array(&declared.ty, structs) {
                        let source_binding = match &field.value {
                            ScalarExpression::Name { name, span } => match builder.binding(name, *span) {
                                Ok(source) => source,
                                Err(error) => { errors.push(error); continue; }
                            },
                            ScalarExpression::Call { .. } => {
                                let point = &builder.cfg.points[member.value.point.0];
                                let CfgPointKind::Call { output, .. } = &point.kind else {
                                    errors.push(builder.missing_context(field.span, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
                                    continue;
                                };
                                ReferenceBindingId { source: source.clone(), function_span: module_span,
                                    declaration_span: point.source_span.range, block_span: module_span,
                                    kind: ReferenceBindingKind::CallOutput { point: member.value.point, output: *output } }
                            }
                            _ => { errors.push(builder.missing_context(field.span, ReferenceAnalysisErrorReason::MissingResolvedBinding)); continue; }
                        };
                        let place = ScalarPlace::Field {
                            base: Box::new(ScalarPlace::Name { name: binding.name.clone(), span: binding.name_span }),
                            field: ScalarFieldReference::Resolved(declared.id.clone()), span: field.span,
                        };
                        match builder.assignment_place(&place, field.span) {
                            Ok(target) => { builder.point(CfgPointKind::CopyProjectedBackings { source: source_binding, target },
                                field.span, scope, BTreeSet::new()); }
                            Err(error) => errors.push(error),
                        }
                    }
                    if !matches!(declared.ty, ScalarType::Callable { .. } | ScalarType::CheckedReference { .. }) {
                        continue;
                    }
                    if matches!(&field.value, ScalarExpression::Name { name, .. } if module_names.contains(name) && !builder.names.contains_key(name))
                    {
                        continue;
                    }
                    let rhs_point = member.value.point;
                    let place = ScalarPlace::Field {
                        base: Box::new(ScalarPlace::Name {
                            name: binding.name.clone(),
                            span: binding.name_span,
                        }),
                        field: ScalarFieldReference::Resolved(declared.id.clone()),
                        span: field.span,
                    };
                    match builder.assignment_place(&place, field.span) {
                        Ok(target) => {
                            let field_assign = builder.point(
                                CfgPointKind::Assign {
                                    target,
                                    rhs_point,
                                    previous_origins: BTreeSet::new(),
                                    previous_loans: Vec::new(),
                                },
                                field.span,
                                scope,
                                BTreeSet::new(),
                            );
                            if let Err(error) = builder.binding_point(
                                binding,
                                0,
                                ScalarBindingPointRole::FieldAssign(declared.id.clone()),
                                field_assign,
                            ) { errors.push(error); }
                        }
                        Err(error) => errors.push(error),
                    }
                }
            }
            if runtime_value {
                if let Err(error) = module_backing_point(
                    &mut builder,
                    &id,
                    &binding.declared_type,
                    &binding.value,
                    rhs_point,
                    binding.is_allocation,
                    binding.name_span,
                    scope,
                ) {
                    errors.push(error);
                }
            }
            if binding.receivers.len() > 1 {
                for (output, receiver) in binding.receivers.iter().enumerate().skip(1) {
                    let (value, point) = if binding.output_origin
                        == ScalarBindingOutputOrigin::IndependentExpressions
                    {
                        let value = &binding.output_values[output].value;
                        match builder.expression(value, scope) {
                            Ok(_) => {
                                let point = builder
                                    .expression_value_point(value, 0)
                                    .expect("actual independent module output");
                                if let Err(error) = builder.move_value(value, scope) {
                                    errors.push(error);
                                }
                                (value, point)
                            }
                            Err(error) => {
                                errors.push(error);
                                continue;
                            }
                        }
                    } else if binding.output_origin == ScalarBindingOutputOrigin::SingleExpression {
                        let retained = match builder.binding_output_source(binding, output) {
                            Ok(retained) => retained,
                            Err(error) => { errors.push(error); continue; }
                        };
                        let point =
                            match output_sequence::retained_source_value(&builder.cfg, &retained, &binding.occurrence.source_span) {
                                Ok(value) => value.point,
                                Err(error) => {
                                    errors.push(error);
                                    continue;
                                }
                            };
                        (&binding.value, point)
                    } else {
                        errors.push(builder.missing_context(
                            receiver.name_span,
                            ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                        ));
                        continue;
                    };
                    let id = ReferenceBindingId {
                        source: source.clone(),
                        function_span: module_span,
                        declaration_span: receiver.name_span,
                        block_span: module_span,
                        kind: ReferenceBindingKind::Declared,
                    };
                    builder.names.insert(receiver.name.clone(), id.clone());
                    builder
                        .binding_types
                        .insert(id.clone(), receiver.ty.clone());
                    if let Err(error) = builder.register_source_binding(&id, &receiver.ty) { errors.push(error); }
                    let receiver_bind =
                        if matches!(receiver.ty, ScalarType::CheckedReference { .. }) {
                            builder.checked_storage(
                                &id,
                                ScalarPlace::Name {
                                    name: receiver.name.clone(),
                                    span: receiver.name_span,
                                },
                            );
                            builder.checked.insert(id.clone());
                            builder.point_with_origins(
                                CfgPointKind::Bind {
                                    binding: id.clone(),
                                },
                                receiver.name_span,
                                scope,
                                None,
                            )
                        } else {
                            builder.point(
                                CfgPointKind::Bind {
                                    binding: id.clone(),
                                },
                                receiver.name_span,
                                scope,
                                BTreeSet::new(),
                            )
                        };
                    if let Err(error) = builder.binding_source_point(binding, output, receiver_bind)
                    {
                        errors.push(error);
                    }
                    if let Err(error) = builder.copy_evaluated_aggregate_binding(binding, &id, output, scope) { errors.push(error); }
                    if matches!(receiver.ty, ScalarType::CheckedReference { .. }) {
                        builder.checked_bind_input(receiver_bind, point);
                    }
                    if matches!(
                        receiver.ty,
                        ScalarType::RuntimeArray { .. } | ScalarType::Array { .. }
                    ) {
                        if let Err(error) = module_backing_point(
                            &mut builder,
                            &id,
                            &receiver.ty,
                            value,
                            Some(point),
                            false,
                            receiver.name_span,
                            scope,
                        ) {
                            errors.push(error);
                        }
                    }
                }
            }
            if let Err(error) = builder.validate_call_receivers(binding) {
                errors.push(error);
            }
        } else {
            if let ScalarItem::Executable(ScalarBlockItem::Assignment(assignment)) = item {
                let output_types = assignment
                    .targets
                    .iter()
                    .map(|target| match &target.place {
                        ScalarPlace::Name { name, .. } => builder
                            .names
                            .get(name)
                            .and_then(|id| builder.binding_types.get(id))
                            .cloned(),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if (assignment.values.len() == 1
                    && assignment.targets.len() > 1
                    && matches!(assignment.values[0], ScalarExpression::Call { .. }))
                    || output_types.iter().any(|ty| {
                        matches!(
                            ty,
                            Some(ScalarType::RuntimeArray { .. } | ScalarType::Array { .. })
                        )
                    })
                {
                    let mut points = Vec::new();
                    if assignment.values.len() == 1
                        && matches!(assignment.values[0], ScalarExpression::Call { .. })
                    {
                        match builder.expression(&assignment.values[0], scope) {
                            Ok(_) => match builder.call_output_sequence(&assignment.values[0]) {
                                Ok(sequence) => points.extend(
                                    sequence.entries.into_iter().map(|entry| entry.value.point),
                                ),
                                Err(error) => {
                                    errors.push(error);
                                    continue;
                                }
                            },
                            Err(error) => {
                                errors.push(error);
                                continue;
                            }
                        }
                    } else {
                        for value in &assignment.values {
                            match builder.expression(value, scope) {
                                Ok(_) => {
                                    points.push(
                                        builder
                                            .expression_value_point(value, 0)
                                            .expect("module assignment value"),
                                    );
                                    if let Err(error) = builder.move_value(value, scope) {
                                        errors.push(error);
                                    }
                                }
                                Err(error) => errors.push(error),
                            }
                        }
                    }
                    for (position, (target, ty)) in
                        assignment.targets.iter().zip(&output_types).enumerate()
                    {
                        let Some(ty) = ty else { continue };
                        let Some(rhs_point) = points.get(position).copied() else {
                            continue;
                        };
                        let value = if assignment.values.len() == 1 {
                            &assignment.values[0]
                        } else {
                            &assignment.values[position]
                        };
                        if matches!(
                            ty,
                            ScalarType::RuntimeArray { .. } | ScalarType::Array { .. }
                        ) {
                            if let ScalarPlace::Name { name, .. } = &target.place {
                                let id = match builder.binding(name, target.target_span) {
                                    Ok(id) => id,
                                    Err(error) => {
                                        errors.push(error);
                                        continue;
                                    }
                                };
                                match builder.assignment_place(&target.place, target.target_span) {
                                    Ok(place) => {
                                        let assigned = builder.point(
                                            CfgPointKind::Assign {
                                                target: place,
                                                rhs_point,
                                                previous_origins: BTreeSet::new(),
                                                previous_loans: Vec::new(),
                                            },
                                            target.target_span,
                                            scope,
                                            BTreeSet::new(),
                                        );
                                        builder.assignment_point(
                                            assignment,
                                            position,
                                            ScalarAssignmentPointRole::Assign,
                                            assigned,
                                        );
                                    }
                                    Err(error) => {
                                        errors.push(error);
                                        continue;
                                    }
                                }
                                if let Err(error) = module_backing_point(
                                    &mut builder,
                                    &id,
                                    ty,
                                    value,
                                    Some(rhs_point),
                                    false,
                                    target.target_span,
                                    scope,
                                ) {
                                    errors.push(error);
                                }
                                if matches!(ty, ScalarType::Array { element, .. } if matches!(element.as_ref(), ScalarType::Callable { .. }))
                                {
                                    if let ScalarExpression::Name {
                                        name: source_name, ..
                                    } = value
                                    {
                                        if let Some(source) =
                                            builder.names.get(source_name).cloned()
                                        {
                                            let copy = builder.point(
                                                CfgPointKind::CopyCallableFields {
                                                    source: CallableFieldSource::Assignment(source),
                                                    target: id.clone(),
                                                },
                                                target.target_span,
                                                scope,
                                                BTreeSet::new(),
                                            );
                                            builder.assignment_point(
                                                assignment,
                                                position,
                                                ScalarAssignmentPointRole::CopyCallableFields,
                                                copy,
                                            );
                                        }
                                    }
                                }
                            }
                        } else if matches!(
                            ty,
                            ScalarType::CheckedReference { .. }
                                | ScalarType::Callable { .. }
                                | ScalarType::Struct(_)
                        ) {
                            let ScalarPlace::Name { name, .. } = &target.place else {
                                continue;
                            };
                            let Some(binding) = builder.names.get(name).cloned() else {
                                errors.push(builder.missing_context(
                                    target.target_span,
                                    ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                ));
                                continue;
                            };
                            let place =
                                match builder.assignment_place(&target.place, target.target_span) {
                                    Ok(place) => place,
                                    Err(error) => {
                                        errors.push(error);
                                        continue;
                                    }
                                };
                            let origins = builder.cfg.points[rhs_point.0].possible_origins.clone();
                            let assigned = builder.point_with_origins(
                                CfgPointKind::Assign {
                                    target: place,
                                    rhs_point,
                                    previous_origins: BTreeSet::new(),
                                    previous_loans: Vec::new(),
                                },
                                target.target_span,
                                scope,
                                if matches!(ty, ScalarType::CheckedReference { .. }) {
                                    origins.clone()
                                } else {
                                    Some(BTreeSet::new())
                                },
                            );
                            builder.assignment_point(
                                assignment,
                                position,
                                ScalarAssignmentPointRole::Assign,
                                assigned,
                            );
                            if matches!(ty, ScalarType::CheckedReference { .. }) {
                                if let Some(origins) = origins {
                                    builder.origins.insert(binding.clone(), origins);
                                } else {
                                    builder.origins.remove(&binding);
                                }
                            }
                            if matches!(ty, ScalarType::Struct(_)) {
                                let source = ReferenceBindingId {
                                    source: source.clone(),
                                    function_span: module_span,
                                    declaration_span: builder.cfg.points[rhs_point.0]
                                        .source_span
                                        .range,
                                    block_span: module_span,
                                    kind: ReferenceBindingKind::CallOutput {
                                        point: rhs_point,
                                        output: position,
                                    },
                                };
                                let copy = builder.point(
                                    CfgPointKind::CopyCallableFields {
                                        source: CallableFieldSource::Assignment(source),
                                        target: binding,
                                    },
                                    target.target_span,
                                    scope,
                                    BTreeSet::new(),
                                );
                                builder.assignment_point(
                                    assignment,
                                    position,
                                    ScalarAssignmentPointRole::CopyCallableFields,
                                    copy,
                                );
                            }
                        }
                    }
                    record_top_level_item_points(&mut builder.cfg, item, start);
                    record_top_level_assignment(
                        &mut builder.cfg,
                        assignment,
                        &builder.names,
                        &builder.binding_types,
                        structs,
                    );
                    continue;
                }
            }
            if let ScalarItem::Executable(ScalarBlockItem::Expression(expression)) = item {
                if matches!(
                    expression,
                    ScalarExpression::Call { .. }
                        | ScalarExpression::Block(_)
                        | ScalarExpression::If { .. }
                        | ScalarExpression::UnitIf { .. }
                ) {
                    if let Err(error) = builder.expression(expression, scope) {
                        errors.push(error);
                    }
                }
            }
            if let ScalarItem::Executable(ScalarBlockItem::Assignment(assignment)) = item {
                for (position, (target, value)) in assignment
                    .targets
                    .iter()
                    .zip(&assignment.values)
                    .enumerate()
                {
                    if let ScalarPlace::Name { name, .. } = &target.place {
                        if let Some(binding) = builder.names.get(name).cloned() {
                            if builder.checked.contains(&binding) {
                                match builder.expression(value, scope) {
                                    Ok(_) => {
                                        let rhs_point = builder
                                            .expression_value_point(value, 0)
                                            .expect("module assignment source output");
                                        let origins = builder.cfg.points[rhs_point.0]
                                            .possible_origins
                                            .clone();
                                        match builder
                                            .assignment_place(&target.place, target.target_span)
                                        {
                                            Ok(place) => {
                                                builder.point_with_origins(
                                                    CfgPointKind::Assign {
                                                        target: place,
                                                        rhs_point,
                                                        previous_origins: BTreeSet::new(),
                                                        previous_loans: Vec::new(),
                                                    },
                                                    target.target_span,
                                                    scope,
                                                    origins.clone(),
                                                );
                                                builder.assignment_point(
                                                    assignment,
                                                    position,
                                                    ScalarAssignmentPointRole::Assign,
                                                    builder
                                                        .cursor
                                                        .expect("module checked assignment"),
                                                );
                                                if let Some(origins) = origins {
                                                    builder.origins.insert(binding, origins);
                                                } else {
                                                    builder.origins.remove(&binding);
                                                }
                                            }
                                            Err(error) => errors.push(error),
                                        }
                                    }
                                    Err(error) => errors.push(error),
                                }
                            } else if matches!(
                                builder.binding_types[&binding],
                                ScalarType::Callable { .. } | ScalarType::Struct(_)
                            ) {
                                match builder.expression(value, scope) {
                                    Ok(_) => {
                                        let rhs_point = builder
                                            .cursor
                                            .expect("module callable assignment value");
                                        match builder
                                            .assignment_place(&target.place, target.target_span)
                                        {
                                            Ok(place) => {
                                                let assigned = builder.point(
                                                    CfgPointKind::Assign {
                                                        target: place,
                                                        rhs_point,
                                                        previous_origins: BTreeSet::new(),
                                                        previous_loans: Vec::new(),
                                                    },
                                                    target.target_span,
                                                    scope,
                                                    BTreeSet::new(),
                                                );
                                                builder.assignment_point(
                                                    assignment,
                                                    position,
                                                    ScalarAssignmentPointRole::Assign,
                                                    assigned,
                                                );
                                                if matches!(
                                                    builder.binding_types[&binding],
                                                    ScalarType::Struct(_)
                                                ) {
                                                    let source_binding = match value {
                                                        ScalarExpression::Name { name, .. } => builder.names.get(name).cloned(),
                                                        ScalarExpression::Call { .. } => {
                                                            match &builder.cfg.points[rhs_point.0].kind {
                                                                CfgPointKind::Call { output, .. } => Some(ReferenceBindingId {
                                                                    source: source.clone(),
                                                                    function_span: module_span,
                                                                    declaration_span: builder.cfg.points[rhs_point.0].source_span.range,
                                                                    block_span: module_span,
                                                                    kind: ReferenceBindingKind::CallOutput { point: rhs_point, output: *output },
                                                                }),
                                                                _ => None,
                                                            }
                                                        }
                                                        _ => None,
                                                    };
                                                    if let Some(source_binding) = source_binding {
                                                        let copy = builder.point(
                                                            CfgPointKind::CopyCallableFields {
                                                                source: CallableFieldSource::Assignment(source_binding),
                                                                target: binding.clone(),
                                                            },
                                                            target.target_span,
                                                            scope,
                                                            BTreeSet::new(),
                                                        );
                                                        builder.assignment_point(assignment, position, ScalarAssignmentPointRole::CopyCallableFields, copy);
                                                    }
                                                }
                                            }
                                            Err(error) => errors.push(error),
                                        }
                                    }
                                    Err(error) => errors.push(error),
                                }
                            }
                        }
                    } else if let ScalarPlace::Field {
                        field: ScalarFieldReference::Resolved(field),
                        ..
                    } = &target.place
                    {
                        let declared = builder
                            .structs
                            .iter()
                            .find(|structure| structure.id == field.structure)
                            .and_then(|structure| {
                                structure
                                    .fields
                                    .iter()
                                    .find(|declared| declared.id == *field)
                            })
                            .map(|declared| declared.ty.clone());
                        let Some(declared) = declared else {
                            errors.push(builder.missing_context(
                                target.target_span,
                                ReferenceAnalysisErrorReason::MissingResolvedBinding,
                            ));
                            continue;
                        };
                        if matches!(
                            declared,
                            ScalarType::Callable { .. }
                                | ScalarType::RuntimeArray { .. }
                                | ScalarType::CheckedReference { .. }
                        ) {
                            let result = (|| -> Result<(), ReferenceAnalysisError> {
                                builder.expression(value, scope)?;
                                let rhs_point = builder.expression_value_point(value, 0)?;
                                builder.place_expressions(&target.place, scope)?;
                                let place =
                                    builder.assignment_place(&target.place, target.target_span)?;
                                let origins =
                                    builder.cfg.points[rhs_point.0].possible_origins.clone();
                                let assigned = builder.point_with_origins(
                                    CfgPointKind::Assign {
                                        target: place.clone(),
                                        rhs_point,
                                        previous_origins: BTreeSet::new(),
                                        previous_loans: Vec::new(),
                                    },
                                    target.target_span,
                                    scope,
                                    if matches!(declared, ScalarType::CheckedReference { .. }) {
                                        origins
                                    } else {
                                        Some(BTreeSet::new())
                                    },
                                );
                                builder.assignment_point(
                                    assignment,
                                    position,
                                    ScalarAssignmentPointRole::Assign,
                                    assigned,
                                );
                                if matches!(declared, ScalarType::RuntimeArray { .. }) {
                                    module_backing_place(
                                        &mut builder,
                                        place,
                                        value,
                                        Some(rhs_point),
                                        target.target_span,
                                        scope,
                                    )?;
                                }
                                Ok(())
                            })();
                            if let Err(error) = result {
                                errors.push(error);
                            }
                        }
                    } else if matches!(&target.place, ScalarPlace::Index { .. })
                        && matches!(
                            module_assignment_place_type(
                                &target.place,
                                &builder.names,
                                &builder.binding_types,
                                structs
                            ),
                            Some(ScalarType::Callable { .. })
                        )
                    {
                        let result = (|| -> Result<(), ReferenceAnalysisError> {
                            builder.expression(value, scope)?;
                            let rhs_point = builder
                                .cursor
                                .expect("evaluated module array element value");
                            builder.place_expressions(&target.place, scope)?;
                            if let Some(selected) = builder.select_callable_array_element(
                                &target.place,
                                target.target_span,
                                scope,
                            )? {
                                builder.assignment_point(
                                    assignment,
                                    position,
                                    ScalarAssignmentPointRole::SelectElement,
                                    selected,
                                );
                            }
                            let place =
                                builder.assignment_place(&target.place, target.target_span)?;
                            let assigned = builder.point(
                                CfgPointKind::Assign {
                                    target: place,
                                    rhs_point,
                                    previous_origins: BTreeSet::new(),
                                    previous_loans: Vec::new(),
                                },
                                target.target_span,
                                scope,
                                BTreeSet::new(),
                            );
                            builder.assignment_point(
                                assignment,
                                position,
                                ScalarAssignmentPointRole::Assign,
                                assigned,
                            );
                            Ok(())
                        })();
                        if let Err(error) = result {
                            errors.push(error);
                        }
                    }
                }
                if builder.cfg.points.len() == start {
                    for value in &assignment.values {
                        if let Err(error) = builder.expression(value, scope) {
                            errors.push(error);
                        }
                    }
                    for target in &assignment.targets {
                        if let Err(error) = builder.place_expressions(&target.place, scope) {
                            errors.push(error);
                        }
                    }
                }
            }
        }
        record_top_level_item_points(&mut builder.cfg, item, start);
        if let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
            item
        {
            record_top_level_block_bindings(&mut builder.cfg, block);
        }
        if let ScalarItem::Executable(ScalarBlockItem::Assignment(assignment)) = item {
            record_top_level_assignment(
                &mut builder.cfg,
                assignment,
                &builder.names,
                &builder.binding_types,
                structs,
            );
        }
    }
    let connected_points = builder.cfg.points.len();
    for entry in &mut builder.cfg.top_level.points {
        if let ScalarTopLevelPointExecution::Connected { outgoing, .. } = &mut entry.execution {
            *outgoing = builder.cfg.points[entry.point.0].successors.clone();
        }
    }
    builder.publish_rebind_associations();
    builder.cfg.initial_points = builder.cfg.points.clone();
    if let Err(error) = name_resolution::complete_module_source(&mut builder.cfg, source, items) { errors.push(error); return errors; }
    if let Err(error) = source_boundary::module_source_topology::freeze_connected(&mut builder.cfg, &placed_captures) { errors.push(error); return errors; }
    if let Err(error) = name_resolution::validate(&builder.cfg, structs) { errors.push(error); return errors; }
    if let Err(error) = selected_source::register_literal_members(&mut builder.cfg, structs, &builder.generic_scope, &builder.generic_declarations) {
        errors.push(error);
    }
    if errors.is_empty() {
        let capture_input = match source_boundary::module_preflight_forest(source, items, &builder.cfg, structs, enums, SourceCaptureConstructionPhase::Building)
            .and_then(|forest| source_boundary::preflight_input(&builder.cfg.producer_owner, &forest, BTreeMap::new())) {
            Ok(input) => input, Err(error) => { errors.push(error); return errors; }
        };
        let initial_transfer = transfer_reference_facts_observed(
            &mut builder.cfg,
            &BTreeMap::new(),
            &[],
            &[],
            &[],
            &capture_input,
            #[cfg(test)]
            ReferenceTransferTestRole::TopLevel,
            #[cfg(test)]
            ReferenceTransferTestStage::InitialConstruction,
            #[cfg(test)]
            trace,
        );
        errors.extend(record_literal_reference_cfgs(
            &mut builder.cfg,
            &capture_input,
            &initial_transfer,
            source,
            module_span,
            &builder.generic_scope,
            &builder.generic_declarations,
            &BTreeSet::new(),
            module_names,
            &BTreeMap::new(),
            call_sites,
            structs,
            enums,
            assignment_outputs,
            literal_signatures,
            value_types,
            #[cfg(test)]
            trace,
        ));
        if errors.is_empty() {
            let completion = source_boundary::module_preflight_forest(source, items, &builder.cfg, structs, enums, SourceCaptureConstructionPhase::Building)
                .and_then(|forest| source_boundary::preflight_input(&builder.cfg.producer_owner, &forest, BTreeMap::new()))
                .and_then(|input| source_boundary::complete_capture_children(&mut builder.cfg, &input));
            if let Err(error) = completion { errors.push(error); }
        }
    }
    let augmentation = match source_boundary::module_source_topology::prepare_augmentation(&builder.cfg) {
        Ok(input) => input, Err(error) => { if errors.is_empty() { errors.push(error); } return errors; },
    };
    for site in call_sites {
        if items.iter().any(|item| match item {
            ScalarItem::Function(function) => {
                function.span.start <= site.source_span.range.start
                    && site.source_span.range.end <= function.span.end
            }
            _ => false,
        }) {
            continue;
        }
        if builder.cfg.points.iter().any(|point| {
            matches!(&point.kind, CfgPointKind::Call { output: 0, .. })
                && point.source_span == site.source_span
        }) {
            continue;
        }
        let id = CfgPointId(builder.cfg.points.len());
        builder.cfg.points.push(ReferenceFlowPoint {
            id,
            kind: CfgPointKind::Call {
                expression: site.expression.clone(),
                output: 0,
                target: site.target.clone(),
                argument_points: Vec::new(),
            },
            source_span: site.source_span.clone(),
            scope,
            possible_origins: Some(BTreeSet::new()),
            possible_environments: BTreeSet::new(),
            possible_projected_environments: Vec::new(),
            possible_callee_environments: BTreeSet::new(),
            possible_backings: Vec::new(),
            possible_projected_backings: Vec::new(),
            backing_argument_paths: Vec::new(),
            global_callable_call_paths: Vec::new(),
            global_callable_output_effects: Vec::new(),
            call_import_paths: Vec::new(),
            successors: Vec::new(),
        });
        builder.cfg.top_level.points.push(ScalarTopLevelPoint {
            point: id,
            execution: ScalarTopLevelPointExecution::SupplementalAnalysisOnly,
        });
    }
    for (index, (occurrence, kind)) in top_level_captures.iter().enumerate() {
        if placed_captures.contains(&index) {
            continue;
        }
        let span = &occurrence.source_span;
        let id = CfgPointId(builder.cfg.points.len());
        let mut kind = kind.clone();
        if let CfgPointKind::Capture { environment, .. } = &mut kind {
            *environment = CallableEnvironmentId {
                literal: span.clone(),
                index: builder.cfg.environments.len(),
            };
            builder.cfg.environments.push(CallableEnvironmentFact {
                literal: span.clone(),
                instance: CallableEnvironmentInstance::Capture,
                origins: BTreeSet::new(),
                owners: BTreeSet::new(),
                loans: BTreeSet::new(),
                backings: Vec::new(),
                inputs: Vec::new(),
                capture_paths: Vec::new(),
            });
        }
        builder.cfg.points.push(ReferenceFlowPoint {
            id,
            kind,
            source_span: span.clone(),
            scope,
            possible_origins: Some(BTreeSet::new()),
            possible_environments: BTreeSet::new(),
            possible_projected_environments: Vec::new(),
            possible_callee_environments: BTreeSet::new(),
            possible_backings: Vec::new(),
            possible_projected_backings: Vec::new(),
            backing_argument_paths: Vec::new(),
            global_callable_call_paths: Vec::new(),
            global_callable_output_effects: Vec::new(),
            call_import_paths: Vec::new(),
            successors: Vec::new(),
        });
        builder.cfg.top_level.points.push(ScalarTopLevelPoint {
            point: id,
            execution: ScalarTopLevelPointExecution::SupplementalAnalysisOnly,
        });
    }
    builder
        .cfg
        .initial_points
        .extend(builder.cfg.points[connected_points..].iter().cloned());
    if let Err(error) = source_boundary::module_source_topology::finish_augmentation(&mut builder.cfg, augmentation) { if errors.is_empty() { errors.push(error); } return errors; }
    *top_level_cfg = builder.cfg;
    errors
}

fn module_capture_point(
    builder: &mut ReferenceFlowBuilder,
    occurrence: &ScalarCaptureOccurrence,
    kind: &CfgPointKind,
    scope: ReferenceScopeId,
) -> Result<(), ReferenceAnalysisError> {
    let span = &occurrence.source_span;
    let mut kind = kind.clone();
    if let CfgPointKind::Capture { environment, .. } = &mut kind {
        *environment = CallableEnvironmentId {
            literal: span.clone(),
            index: builder.cfg.environments.len(),
        };
        builder.cfg.environments.push(CallableEnvironmentFact {
            literal: span.clone(),
            instance: CallableEnvironmentInstance::Capture,
            origins: BTreeSet::new(),
            owners: BTreeSet::new(),
            loans: BTreeSet::new(),
            backings: Vec::new(),
            inputs: Vec::new(),
            capture_paths: Vec::new(),
        });
    }
    let point = builder.point(kind, span.range, scope, BTreeSet::new());
    builder.cfg.capture_points.push(ScalarCaptureCfgPoint {
        occurrence: occurrence.clone(),
        role: ScalarCapturePointRole::Capture,
        point,
    });
    builder.register_capture_input(point)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ReferenceCallableBody {
    Declaration,
    Overload(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReferenceOutputContract {
    source: SourceIdentity,
    declaration_span: ByteSpan,
    body: ReferenceCallableBody,
    output: usize,
    origins: Vec<ReferenceOrigin>,
    pub(super) environments: Vec<LiteralResultEnvironment>,
    pub(super) projected: Vec<(ReferencePlaceId, LiteralResultEnvironment)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConcreteReferenceOutputContract {
    source: SourceIdentity,
    declaration_span: ByteSpan,
    selection: ConcreteCallSelection,
    type_arguments: Vec<ScalarType>,
    output: usize,
    origins: Vec<ReferenceOrigin>,
    environments: Vec<LiteralResultEnvironment>,
    projected: Vec<(ReferencePlaceId, LiteralResultEnvironment)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LiteralOutputContract {
    literal: SourceSpan,
    output: usize,
    origins: Vec<ReferenceOrigin>,
    pub(super) environments: Vec<LiteralResultEnvironment>,
    pub(super) projected: Vec<(ReferencePlaceId, LiteralResultEnvironment)>,
    runtime_array: bool,
    backings: Vec<RuntimeOutputBackingSource>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct LiteralResultEnvironment {
    pub(super) producer: ProducerReturnSelectionFact,
    pub(super) id: CallableEnvironmentId,
    facts: Vec<ReferenceOrigin>,
    environments: Vec<CallableEnvironmentFact>,
}

pub(super) fn producer_cfg_source(owner: &ProducerCfgOwnerId) -> &SourceIdentity {
    match owner {
        ProducerCfgOwnerId::Module(source) | ProducerCfgOwnerId::Function { source, .. } => source,
        ProducerCfgOwnerId::Literal { enclosing, .. } => producer_cfg_source(enclosing),
    }
}

fn result_environment(
    cfg: &ScalarReferenceCfg,
    point: &ReferenceFlowPoint,
    output: usize,
    id: &CallableEnvironmentId,
) -> Result<LiteralResultEnvironment, ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: point.source_span.clone(),
        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
    };
    let CfgPointKind::ReturnOutput {
        output: recorded, ..
    } = &point.kind
    else {
        return Err(invalid());
    };
    if *recorded != output
        || cfg.points.get(point.id.0) != Some(point)
        || producer_cfg_source(&cfg.producer_owner) != &point.source_span.source
    {
        return Err(invalid());
    }
    let fact = cfg.environments.get(id.index).ok_or_else(invalid)?;
    if fact.literal != id.literal
        || (!point.possible_environments.contains(id)
            && !point
                .possible_projected_environments
                .iter()
                .any(|projection| projection.environments.contains(id)))
    {
        return Err(invalid());
    }
    if !return_environment_from_inputs(cfg, id)
        && cfg
            .producer_return_cases
            .iter()
            .any(|known| known.return_point == point.id && known.output == output)
        && !cfg.producer_return_cases.iter().any(|known| {
            known.return_point == point.id
                && known.output == output
                && known.table.owner == cfg.producer_owner
                && known.environment == *id
                && known.alternative.source_index == id.index
                && known.alternative.source_instance == fact.instance
        })
    {
        return Err(invalid());
    }
    Ok(LiteralResultEnvironment {
        producer: ProducerReturnSelectionFact {
            table: ProducerTableId {
                owner: cfg.producer_owner.clone(),
            },
            return_point: point.id,
            output,
            alternative: ProducerOutputAlternativeId {
                source_index: id.index,
                source_instance: fact.instance.clone(),
            },
        },
        id: id.clone(),
        facts: cfg.facts.clone(),
        environments: cfg.environments.clone(),
    })
}

fn collect_literal_output_contracts(
    cfg: &ScalarReferenceCfg,
    contracts: &mut Vec<LiteralOutputContract>,
) -> Result<(), ReferenceAnalysisError> {
    for literal in &cfg.literal_cfgs {
        let body = &literal.reference_cfg;
        let capture = cfg
            .initial_points
            .iter()
            .find_map(|point| match &point.kind {
                CfgPointKind::Capture {
                    environment,
                    signature: ScalarType::Callable { outputs, .. },
                    parameter_spans,
                    ..
                } if environment.literal == literal.literal => Some((outputs, parameter_spans)),
                _ => None,
            })
            .expect("recorded literal capture");
        for point in &body.points {
            let CfgPointKind::ReturnOutput {
                output,
                ref backing_action,
                value_point,
                ..
            } = point.kind
            else {
                continue;
            };
            let runtime_array = matches!(
                capture.0.outputs[output].ty,
                ScalarType::RuntimeArray { .. }
            );
            let mut backings = Vec::new();
            if runtime_array {
                if let CfgPointKind::Read { binding } = &body.initial_points[value_point.0].kind {
                    if let Some(index) = capture.1.iter().position(|span| {
                        *span == binding.declaration_span
                            && binding.function_span == literal.literal.range
                    }) {
                        backings.push(RuntimeOutputBackingSource::Parameter(index));
                    }
                }
                for fact in &point.possible_backings {
                    let candidate = if let Some(index) = body
                        .backing_input_paths
                        .iter()
                        .flat_map(|path| path.iter())
                        .find(|input| input.value == fact.value)
                        .and_then(|input| {
                            capture.1.iter().position(|span| {
                                *span == input.owner.declaration_span
                                    && input.owner.function_span == literal.literal.range
                            })
                        }) {
                        RuntimeOutputBackingSource::Parameter(index)
                    } else if let RuntimeValuePosition::Parameter { declaration_span } =
                        fact.value.position
                    {
                        capture
                            .1
                            .iter()
                            .position(|span| *span == declaration_span)
                            .map(RuntimeOutputBackingSource::Parameter)
                            .expect("typed literal parameter backing")
                    } else {
                        RuntimeOutputBackingSource::Created {
                            origin: fact.origin.clone(),
                            action: match backing_action {
                                RuntimeReturnBackingAction::CopyFixed { .. } => {
                                    RuntimeOutputBackingAction::CopyFixed
                                }
                                RuntimeReturnBackingAction::MoveFixed { .. } => {
                                    RuntimeOutputBackingAction::MoveFixed
                                }
                                RuntimeReturnBackingAction::Transfer => {
                                    RuntimeOutputBackingAction::Allocate
                                }
                            },
                            lineage: fact.value.clone(),
                            length: fact.length.clone(),
                            callable_family: fact.callable_family.clone(),
                            owner: fact.owner.clone(),
                        }
                    };
                    if !backings.contains(&candidate) {
                        backings.push(candidate);
                    }
                }
            }
            let reference_output = matches!(
                capture.0.outputs[output].ty,
                ScalarType::CheckedReference { .. }
                    | ScalarType::Callable { .. }
                    | ScalarType::Struct(_)
            );
            let contract = LiteralOutputContract {
                literal: literal.literal.clone(),
                output,
                runtime_array,
                backings,
                origins: if reference_output {
                    point
                        .possible_origins
                        .iter()
                        .flat_map(|origins| origins.iter().map(|id| body.facts[id.0].clone()))
                        .collect()
                } else {
                    Vec::new()
                },
                environments: point
                    .possible_environments
                    .iter()
                    .map(|id| result_environment(body, point, output, id))
                    .collect::<Result<Vec<_>, _>>()?,
                projected: point
                    .possible_projected_environments
                    .iter()
                    .flat_map(|known| {
                        known.environments.iter().map(|id| {
                            result_environment(body, point, output, id)
                                .map(|result| (known.place.clone(), result))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            if let Some(existing) = contracts.iter_mut().find(|candidate| {
                candidate.literal == contract.literal && candidate.output == output
            }) {
                for origin in contract.origins {
                    if !existing.origins.contains(&origin) {
                        existing.origins.push(origin);
                    }
                }
                for environment in contract.environments {
                    if !existing.environments.contains(&environment) {
                        existing.environments.push(environment);
                    }
                }
                for projection in contract.projected {
                    if !existing.projected.contains(&projection) {
                        existing.projected.push(projection);
                    }
                }
                for backing in contract.backings {
                    if !existing.backings.contains(&backing) {
                        existing.backings.push(backing);
                    }
                }
            } else {
                contracts.push(contract);
            }
        }
        collect_literal_output_contracts(body, contracts)?;
    }
    Ok(())
}

pub(super) fn collect_project_literal_output_contracts(
    modules: &[ScalarModule],
) -> Result<Vec<LiteralOutputContract>, ReferenceAnalysisError> {
    let mut contracts = Vec::new();
    for module in modules {
        collect_literal_output_contracts(&module.top_level_reference_cfg, &mut contracts)?;
        for item in &module.items {
            if let ScalarItem::Function(function) = item {
                collect_function_literal_output_contracts(function, &mut contracts)?;
            }
        }
    }
    Ok(contracts)
}

fn collect_function_literal_output_contracts(
    function: &ScalarFunction,
    contracts: &mut Vec<LiteralOutputContract>,
) -> Result<(), ReferenceAnalysisError> {
    collect_literal_output_contracts(&function.reference_cfg, contracts)?;
    for arm in &function.overload_arms {
        collect_function_literal_output_contracts(arm, contracts)?;
    }
    Ok(())
}

fn remap_import_origins(
    origins: &[ReferenceOriginId],
    set: &BTreeSet<ReferenceOriginId>,
    span: &SourceSpan,
) -> Result<BTreeSet<ReferenceOriginId>, ReferenceAnalysisError> {
    set.iter()
        .map(|origin| {
            origins
                .get(origin.0)
                .copied()
                .ok_or_else(|| ReferenceAnalysisError::MissingContext {
                    source_span: span.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                })
        })
        .collect()
}

fn remap_import_loans(
    origins: &[ReferenceOriginId],
    loans: &BTreeSet<CallableLoanId>,
    span: &SourceSpan,
) -> Result<BTreeSet<CallableLoanId>, ReferenceAnalysisError> {
    loans
        .iter()
        .map(|loan| {
            origins
                .get(loan.origin.0)
                .map(|origin| CallableLoanId { origin: *origin })
                .ok_or_else(|| ReferenceAnalysisError::MissingContext {
                    source_span: span.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                })
        })
        .collect()
}

fn import_cell_domain(
    cfg: &mut ScalarReferenceCfg,
    result: &LiteralResultEnvironment,
    cell: &CallableCellDomainFact,
    origins: &[ReferenceOriginId],
    instance: &CallableEnvironmentInstance,
    visiting: &mut BTreeSet<usize>,
    span: &SourceSpan,
) -> Result<CallableCellDomainFact, ReferenceAnalysisError> {
    let mut alternatives = Vec::new();
    for case in &cell.alternatives {
        let rebased = match case {
            CallableCellCaseFact::Uninitialized => CallableCellCaseFact::Uninitialized,
            CallableCellCaseFact::Initialized {
                environment_origin,
                instance: cell_instance,
                holder_aliases,
                backing,
                owner,
                capture_paths,
                loans,
                moved,
                origin,
            } => {
                let environment_origin = import_result_environment(
                    cfg,
                    result,
                    environment_origin,
                    instance,
                    visiting,
                    span,
                )?;
                let mut rebased_paths = Vec::new();
                for path in capture_paths {
                    rebased_paths.push(
                        path.iter()
                            .map(|input| {
                                import_capture_input_nested(
                                    cfg, result, input, origins, instance, visiting, span,
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                }
                CallableCellCaseFact::Initialized {
                    environment_origin,
                    instance: cell_instance.clone(),
                    holder_aliases: holder_aliases.clone(),
                    backing: backing.clone(),
                    owner: owner.clone(),
                    capture_paths: rebased_paths,
                    loans: remap_import_loans(origins, loans, span)?,
                    moved: *moved,
                    origin: origin.clone(),
                }
            }
        };
        if !alternatives.contains(&rebased) {
            alternatives.push(rebased);
        }
    }
    Ok(CallableCellDomainFact { alternatives })
}

fn import_backing_family(
    cfg: &mut ScalarReferenceCfg,
    result: &LiteralResultEnvironment,
    backing: &RuntimeBackingFact,
    origins: &[ReferenceOriginId],
    instance: &CallableEnvironmentInstance,
    visiting: &mut BTreeSet<usize>,
    span: &SourceSpan,
) -> Result<RuntimeBackingFact, ReferenceAnalysisError> {
    let mut rebased = backing.clone();
    for family in &mut rebased.callable_family {
        for element in &mut family.elements {
            let environment = element.environment.clone();
            element.environment =
                import_result_environment(cfg, result, &environment, instance, visiting, span)?;
            element.loans = remap_import_loans(origins, &element.loans, span)?;
        }
        for partition in &mut family.partitions {
            for representative in &mut partition.representatives {
                representative.cell = import_cell_domain(
                    cfg,
                    result,
                    &representative.cell.clone(),
                    origins,
                    instance,
                    visiting,
                    span,
                )?;
            }
            partition.remainder = import_cell_domain(
                cfg,
                result,
                &partition.remainder.clone(),
                origins,
                instance,
                visiting,
                span,
            )?;
        }
    }
    Ok(rebased)
}

fn import_projected_backing(
    cfg: &mut ScalarReferenceCfg,
    result: &LiteralResultEnvironment,
    projected: &RuntimeProjectedBackingFact,
    origins: &[ReferenceOriginId],
    instance: &CallableEnvironmentInstance,
    visiting: &mut BTreeSet<usize>,
    span: &SourceSpan,
) -> Result<RuntimeProjectedBackingFact, ReferenceAnalysisError> {
    Ok(RuntimeProjectedBackingFact {
        place: projected.place.clone(),
        backing: import_backing_family(
            cfg,
            result,
            &projected.backing,
            origins,
            instance,
            visiting,
            span,
        )?,
    })
}

fn import_capture_input_nested(
    cfg: &mut ScalarReferenceCfg,
    result: &LiteralResultEnvironment,
    input: &CallableCaptureInput,
    origins: &[ReferenceOriginId],
    instance: &CallableEnvironmentInstance,
    visiting: &mut BTreeSet<usize>,
    span: &SourceSpan,
) -> Result<CallableCaptureInput, ReferenceAnalysisError> {
    let environments = input
        .environments
        .iter()
        .map(|environment| {
            import_result_environment(cfg, result, environment, instance, visiting, span)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let projected = input
        .projected
        .iter()
        .map(|projection| {
            let environments = projection
                .environments
                .iter()
                .map(|environment| {
                    import_result_environment(cfg, result, environment, instance, visiting, span)
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            Ok(CallableProjectedEnvironment {
                place: projection.place.clone(),
                environments,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let backings = input
        .backings
        .iter()
        .map(|backing| {
            import_backing_family(cfg, result, backing, origins, instance, visiting, span)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let projected_backings = input
        .projected_backings
        .iter()
        .map(|projected| {
            import_projected_backing(cfg, result, projected, origins, instance, visiting, span)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CallableCaptureInput {
        binding: input.binding.clone(),
        origins: remap_import_origins(origins, &input.origins, span)?,
        environments,
        projected,
        backings,
        projected_backings,
    })
}

pub(super) fn import_result_environment(
    cfg: &mut ScalarReferenceCfg,
    result: &LiteralResultEnvironment,
    source: &CallableEnvironmentId,
    instance: &CallableEnvironmentInstance,
    visiting: &mut BTreeSet<usize>,
    span: &SourceSpan,
) -> Result<CallableEnvironmentId, ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: span.clone(),
        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
    };
    let template = result.environments.get(source.index).ok_or_else(invalid)?;
    if template.literal != source.literal {
        return Err(invalid());
    }
    let key = EnvironmentImportFact {
        table: result.producer.table.clone(),
        source_index: source.index,
        source_instance: template.instance.clone(),
        destination_instance: instance.clone(),
        destination: CallableEnvironmentId {
            literal: source.literal.clone(),
            index: 0,
        },
    };
    let target_index = match cfg.environment_imports.iter().find(|known| {
        known.table == key.table
            && known.source_index == key.source_index
            && known.source_instance == key.source_instance
            && known.destination_instance == key.destination_instance
    }) {
        Some(known) => {
            let actual = cfg
                .environments
                .get(known.destination.index)
                .ok_or_else(invalid)?;
            if known.destination.literal != template.literal
                || actual.literal != template.literal
                || actual.instance != *instance
            {
                return Err(invalid());
            }
            known.destination.index
        }
        None => {
            let index = cfg.environments.len();
            cfg.environments.push(CallableEnvironmentFact {
                literal: template.literal.clone(),
                instance: instance.clone(),
                origins: BTreeSet::new(),
                owners: BTreeSet::new(),
                loans: BTreeSet::new(),
                backings: Vec::new(),
                inputs: Vec::new(),
                capture_paths: Vec::new(),
            });
            cfg.environment_imports.push(EnvironmentImportFact {
                destination: CallableEnvironmentId {
                    literal: template.literal.clone(),
                    index,
                },
                ..key
            });
            index
        }
    };
    let mut origins = Vec::new();
    for origin in &result.facts {
        let id = match cfg.facts.iter().position(|known| known == origin) {
            Some(index) => ReferenceOriginId(index),
            None => {
                let index = cfg.facts.len();
                cfg.facts.push(origin.clone());
                ReferenceOriginId(index)
            }
        };
        origins.push(id);
    }
    let id = CallableEnvironmentId {
        literal: template.literal.clone(),
        index: target_index,
    };
    if !visiting.insert(source.index) {
        return Ok(id);
    }
    let mut rebased_backings = Vec::new();
    for backing in &template.backings {
        rebased_backings.push(import_backing_family(
            cfg, result, backing, &origins, instance, visiting, span,
        )?);
    }
    let mut rebased_paths = Vec::new();
    for path in &template.capture_paths {
        rebased_paths.push(
            path.iter()
                .map(|input| {
                    import_capture_input_nested(
                        cfg, result, input, &origins, instance, visiting, span,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    let fact = &mut cfg.environments[target_index];
    for rebased in rebased_backings {
        if !fact.backings.contains(&rebased) {
            fact.backings.push(rebased);
        }
    }
    fact.owners.extend(template.owners.iter().cloned());
    fact.origins
        .extend(remap_import_origins(&origins, &template.origins, span)?);
    fact.loans
        .extend(remap_import_loans(&origins, &template.loans, span)?);
    {
        let fact = &mut cfg.environments[target_index];
        for rebased in rebased_paths {
            if !fact.capture_paths.contains(&rebased) {
                fact.capture_paths.push(rebased);
            }
        }
    }
    let mut rebased_input_backings = Vec::new();
    let mut rebased_input_origins = Vec::new();
    for input in &template.inputs {
        rebased_input_origins.push(remap_import_origins(&origins, &input.origins, span)?);
        let mut backings = Vec::new();
        for backing in &input.backings {
            backings.push(import_backing_family(
                cfg, result, backing, &origins, instance, visiting, span,
            )?);
        }
        rebased_input_backings.push(backings);
    }
    for ((input, origins_set), backings) in template
        .inputs
        .iter()
        .zip(rebased_input_origins)
        .zip(rebased_input_backings)
    {
        let fact = &mut cfg.environments[target_index];
        let position = match fact
            .inputs
            .iter()
            .position(|known| known.binding == input.binding)
        {
            Some(index) => index,
            None => {
                let index = fact.inputs.len();
                fact.inputs.push(CallableCaptureInput {
                    binding: input.binding.clone(),
                    origins: BTreeSet::new(),
                    environments: BTreeSet::new(),
                    projected: Vec::new(),
                    backings: Vec::new(),
                    projected_backings: Vec::new(),
                });
                index
            }
        };
        fact.inputs[position].origins.extend(origins_set);
        for rebased in backings {
            if !fact.inputs[position].backings.contains(&rebased) {
                fact.inputs[position].backings.push(rebased);
            }
        }
    }
    for input in &template.inputs {
        let environments = input
            .environments
            .iter()
            .map(|environment| {
                import_result_environment(cfg, result, environment, instance, visiting, span)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let projected = input
            .projected
            .iter()
            .map(|projection| {
                let environments = projection
                    .environments
                    .iter()
                    .map(|environment| {
                        import_result_environment(
                            cfg,
                            result,
                            environment,
                            instance,
                            visiting,
                            span,
                        )
                    })
                    .collect::<Result<BTreeSet<_>, _>>()?;
                Ok(CallableProjectedEnvironment {
                    place: projection.place.clone(),
                    environments,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let captured = cfg.environments[target_index]
            .inputs
            .iter_mut()
            .find(|known| known.binding == input.binding)
            .expect("imported capture input");
        captured.environments.extend(environments);
        for projection in projected {
            if let Some(known) = captured
                .projected
                .iter_mut()
                .find(|known| reference_places_same(&known.place, &projection.place))
            {
                known.environments.extend(projection.environments);
            } else {
                captured.projected.push(projection);
            }
        }
    }
    visiting.remove(&source.index);
    Ok(id)
}

fn import_call_result(
    cfg: &mut ScalarReferenceCfg,
    result: &LiteralResultEnvironment,
    instance: &CallableEnvironmentInstance,
    call: CfgPointId,
    call_kind: &CfgPointKind,
    projection: Vec<ReferencePlaceProjection>,
    selected: &mut Vec<ProducerCallSelectionFact>,
    errors: &mut Vec<ReferenceAnalysisError>,
) -> Option<CallableEnvironmentId> {
    let span = cfg.points[call.0].source_span.clone();
    let CfgPointKind::Call { target, output, .. } = call_kind else {
        unreachable!("call result import requires Call point")
    };
    if result.producer.output != *output
        || result.producer.alternative.source_index != result.id.index
        || result.environments.get(result.id.index).is_none_or(|fact| {
            fact.literal != result.id.literal
                || fact.instance != result.producer.alternative.source_instance
        })
    {
        errors.push(ReferenceAnalysisError::MissingContext {
            source_span: span,
            reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
        });
        return None;
    }
    let destination = match import_result_environment(
        cfg,
        result,
        &result.id,
        instance,
        &mut BTreeSet::new(),
        &span,
    ) {
        Ok(destination) => destination,
        Err(error) => {
            errors.push(error);
            return None;
        }
    };
    let fact = ProducerCallSelectionFact {
        call,
        target: target.clone(),
        output: *output,
        source: result.producer.clone(),
        destination: destination.clone(),
        projection,
    };
    if !selected.contains(&fact) {
        selected.push(fact);
    }
    Some(destination)
}

fn callee_produces_return(
    cfg: &ScalarReferenceCfg,
    callee: &CallableEnvironmentId,
    result: &LiteralResultEnvironment,
) -> bool {
    let ProducerCfgOwnerId::Literal {
        enclosing,
        capture_point,
        literal,
    } = &result.producer.table.owner
    else {
        return false;
    };
    if literal != &callee.literal {
        return false;
    }
    cfg.environment_imports
        .iter()
        .any(|imported| imported.destination == *callee && imported.table.owner == **enclosing)
        || (**enclosing == cfg.producer_owner
            && cfg
                .initial_points
                .get(capture_point.0)
                .is_some_and(|point| {
                    matches!(
                        &point.kind,
                        CfgPointKind::Capture { environment, .. } if environment == callee
                    )
                }))
}

fn literal_callable_parameter_position(
    initial_points: &[ReferenceFlowPoint],
    value: &RuntimeValueId,
) -> Option<usize> {
    let declaration_span = match value.position {
        RuntimeValuePosition::Parameter { declaration_span } => declaration_span,
        _ => return None,
    };
    initial_points.iter().find_map(|point| match &point.kind {
        CfgPointKind::Capture {
            environment,
            parameter_spans,
            ..
        } if environment.literal.source == value.source
            && environment.literal.range == value.function_span =>
        {
            parameter_spans
                .iter()
                .position(|span| *span == declaration_span)
        }
        _ => None,
    })
}

fn substitute_forwarded_param_backing(
    initial_points: &[ReferenceFlowPoint],
    backing: &RuntimeBackingFact,
    module_parameters: Option<&[ReferenceBindingId]>,
    paths: &[Vec<RuntimeCallArgumentFact>],
) -> Vec<RuntimeBackingFact> {
    let position = match module_parameters {
        Some(parameters) => match &backing.value.position {
            RuntimeValuePosition::Parameter { declaration_span } => {
                parameters.iter().position(|binding| {
                    binding.source == backing.value.source
                        && binding.function_span == backing.value.function_span
                        && binding.declaration_span == *declaration_span
                })
            }
            _ => None,
        },
        None => literal_callable_parameter_position(initial_points, &backing.value),
    };
    let Some(position) = position else {
        return vec![backing.clone()];
    };
    let mut forwarded = Vec::new();
    for path in paths {
        if let Some(argument) = path.iter().find(|input| input.position == position) {
            let mut next = backing.clone();
            next.value = argument.backing.value.clone();
            next.kind = argument.backing.kind.clone();
            next.origin = argument.backing.origin.clone();
            next.length = argument.backing.length.clone();
            if !forwarded.contains(&next) {
                forwarded.push(next);
            }
        }
    }
    if forwarded.is_empty() {
        vec![backing.clone()]
    } else {
        forwarded
    }
}

fn forward_imported_capture_backings(
    initial_points: &[ReferenceFlowPoint],
    list: &mut Vec<RuntimeBackingFact>,
    module_parameters: Option<&[ReferenceBindingId]>,
    paths: &[Vec<RuntimeCallArgumentFact>],
) {
    let mut result = Vec::new();
    for backing in list.drain(..) {
        for next in
            substitute_forwarded_param_backing(initial_points, &backing, module_parameters, paths)
        {
            if !result.contains(&next) {
                result.push(next);
            }
        }
    }
    *list = result;
}

fn forward_imported_environment_backings(
    cfg: &mut ScalarReferenceCfg,
    state: &ReferenceEdgeState,
    call_span: &SourceSpan,
    fresh_paths: &[Vec<RuntimeCallArgumentFact>],
    module_parameters: Option<&[ReferenceBindingId]>,
    imported: &BTreeSet<CallableEnvironmentId>,
) {
    if imported.is_empty() {
        return;
    }
    let mut paths: Vec<Vec<RuntimeCallArgumentFact>> = fresh_paths.to_vec();
    if paths.is_empty() {
        for (span, call_paths) in &state.backing_call_paths {
            if span == call_span {
                paths.extend(call_paths.clone());
            }
        }
    }
    if paths.is_empty() {
        return;
    }
    for id in imported {
        let ScalarReferenceCfg {
            initial_points,
            environments,
            ..
        } = &mut *cfg;
        let fact = &mut environments[id.index];
        forward_imported_capture_backings(
            initial_points,
            &mut fact.backings,
            module_parameters,
            &paths,
        );
        for input in &mut fact.inputs {
            forward_imported_capture_backings(
                initial_points,
                &mut input.backings,
                module_parameters,
                &paths,
            );
            let mut projected = Vec::new();
            for known in input.projected_backings.drain(..) {
                for backing in substitute_forwarded_param_backing(
                    initial_points,
                    &known.backing,
                    module_parameters,
                    &paths,
                ) {
                    let candidate = RuntimeProjectedBackingFact {
                        place: known.place.clone(),
                        backing,
                    };
                    if !projected.contains(&candidate) {
                        projected.push(candidate);
                    }
                }
            }
            input.projected_backings = projected;
        }
        for path in &mut fact.capture_paths {
            for input in path.iter_mut() {
                forward_imported_capture_backings(
                    initial_points,
                    &mut input.backings,
                    module_parameters,
                    &paths,
                );
            }
        }
        let mut unique_paths = Vec::new();
        for path in fact.capture_paths.drain(..) {
            if !unique_paths.contains(&path) {
                unique_paths.push(path);
            }
        }
        fact.capture_paths = unique_paths;
    }
}

fn substitute_imported_param_loans(
    cfg: &mut ScalarReferenceCfg,
    state: &ReferenceEdgeState,
    argument_points: &[CfgPointId],
    module_parameters: &[ReferenceBindingId],
    instance: &CallableEnvironmentInstance,
    imported: &BTreeSet<CallableEnvironmentId>,
) {
    if imported.is_empty() || module_parameters.is_empty() {
        return;
    }
    let mut substitutions = Vec::<(ReferenceOriginId, Vec<ReferenceOriginId>)>::new();
    let mut seen = BTreeSet::new();
    for (index, fact) in cfg.environments.iter().enumerate() {
        if fact.instance != *instance
            && !imported.contains(&CallableEnvironmentId {
                literal: fact.literal.clone(),
                index,
            })
        {
            continue;
        }
        let mut candidates = fact.origins.iter().collect::<Vec<_>>();
        for input in &fact.inputs {
            candidates.extend(input.origins.iter());
        }
        for path in &fact.capture_paths {
            for input in path {
                candidates.extend(input.origins.iter());
            }
        }
        for origin in candidates {
            if !seen.insert(*origin) {
                continue;
            }
            let ReferenceOrigin::BorrowedFrom { parameter, loan } = &cfg.facts[origin.0] else {
                continue;
            };
            if !module_parameters.contains(&loan.origin_place.binding) {
                continue;
            }
            let Some(replacements) = argument_points
                .get(*parameter)
                .and_then(|point| state.point_values[point.0].clone())
                .filter(|origins| !origins.is_empty())
            else {
                continue;
            };
            substitutions.push((*origin, replacements.into_iter().collect()));
        }
    }
    if substitutions.is_empty() {
        return;
    }
    for (index, fact) in cfg.environments.iter_mut().enumerate() {
        if fact.instance != *instance
            && !imported.contains(&CallableEnvironmentId {
                literal: fact.literal.clone(),
                index,
            })
        {
            continue;
        }
        let mut remapped = false;
        for (old, replacements) in &substitutions {
            let remap = |origins: &mut BTreeSet<ReferenceOriginId>| {
                if origins.remove(old) {
                    origins.extend(replacements.iter().copied());
                    true
                } else {
                    false
                }
            };
            remapped |= remap(&mut fact.origins);
            for input in &mut fact.inputs {
                remapped |= remap(&mut input.origins);
            }
            for path in &mut fact.capture_paths {
                for input in path.iter_mut() {
                    remapped |= remap(&mut input.origins);
                }
            }
            if fact.loans.remove(&CallableLoanId { origin: *old }) {
                fact.loans.extend(
                    replacements
                        .iter()
                        .map(|origin| CallableLoanId { origin: *origin }),
                );
                remapped = true;
            }
        }
        if remapped {
            let mut unique_paths = Vec::new();
            for path in fact.capture_paths.drain(..) {
                if !unique_paths.contains(&path) {
                    unique_paths.push(path);
                }
            }
            fact.capture_paths = unique_paths;
        }
    }
}

fn merge_call_projected(
    projected: Vec<CallableProjectedEnvironment>,
) -> Vec<CallableProjectedEnvironment> {
    let mut merged: Vec<CallableProjectedEnvironment> = Vec::new();
    for projection in projected {
        if let Some(known) = merged
            .iter_mut()
            .find(|known| reference_places_same(&known.place, &projection.place))
        {
            known.environments.extend(projection.environments);
        } else {
            merged.push(projection);
        }
    }
    merged
}

fn module_callable_parameter_bindings(
    cfg: &ScalarReferenceCfg,
    source: &SourceIdentity,
    declaration_span: ByteSpan,
    concrete: &ConcreteCallSelection,
) -> Vec<ReferenceBindingId> {
    let selection = match concrete {
        ConcreteCallSelection::Declaration => RuntimeCallSelection::Declaration,
        ConcreteCallSelection::Overload(selection) => {
            RuntimeCallSelection::Overload(selection.arm_index)
        }
    };
    cfg.call_signatures
        .iter()
        .find(|signature| {
            signature.source == *source
                && signature.declaration_span == declaration_span
                && signature.selection == selection
        })
        .map(|signature| signature.bindings.clone())
        .unwrap_or_default()
}

fn collect_concrete_output_contracts(
    function: &ScalarFunction,
    templates: &[ReferenceOutputContract],
    concrete: &mut Vec<ConcreteReferenceOutputContract>,
) {
    for arm in &function.overload_arms {
        collect_concrete_output_contracts(arm, templates, concrete);
    }
    collect_cfg_concrete_output_contracts(&function.reference_cfg, templates, concrete);
}

fn collect_cfg_concrete_output_contracts(
    cfg: &ScalarReferenceCfg,
    templates: &[ReferenceOutputContract],
    concrete: &mut Vec<ConcreteReferenceOutputContract>,
) {
    for point in &cfg.initial_points {
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
            environments: template.environments.clone(),
            projected: template.projected.clone(),
        };
        if !concrete.contains(&contract) {
            concrete.push(contract);
        }
    }
    for literal in &cfg.literal_cfgs {
        collect_cfg_concrete_output_contracts(&literal.reference_cfg, templates, concrete);
    }
}

pub(super) fn collect_function_output_contracts(
    source: &SourceIdentity,
    function: &ScalarFunction,
    declaration_span: ByteSpan,
    body: ReferenceCallableBody,
    contracts: &mut Vec<ReferenceOutputContract>,
) -> Result<(), ReferenceAnalysisError> {
    let expected_owner = ProducerCfgOwnerId::Function {
        source: source.clone(),
        declaration: declaration_span,
        body: match body {
            ReferenceCallableBody::Declaration => ProducerFunctionBodyId::Declaration,
            ReferenceCallableBody::Overload(index) => ProducerFunctionBodyId::Overload(index),
        },
    };
    if function.reference_cfg.producer_owner != expected_owner {
        return Err(ReferenceAnalysisError::MissingContext {
            source_span: SourceSpan::new(source.clone(), declaration_span),
            reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
        });
    }
    for point in &function.reference_cfg.points {
        let CfgPointKind::ReturnOutput { output, .. } = point.kind else {
            continue;
        };
        let ScalarType::Callable { outputs, .. } = &function.signature else {
            continue;
        };
        let ty = &outputs.outputs[output].ty;
        if !matches!(
            ty,
            ScalarType::CheckedReference { .. }
                | ScalarType::Callable { .. }
                | ScalarType::Struct(_)
        ) {
            continue;
        }
        let cfg = &function.reference_cfg;
        let result = |id: &CallableEnvironmentId| result_environment(cfg, point, output, id);
        contracts.push(ReferenceOutputContract {
            source: source.clone(),
            declaration_span,
            body: body.clone(),
            output,
            origins: point
                .possible_origins
                .iter()
                .flatten()
                .map(|id| cfg.facts[id.0].clone())
                .collect(),
            environments: point
                .possible_environments
                .iter()
                .map(result)
                .collect::<Result<Vec<_>, _>>()?,
            projected: point
                .possible_projected_environments
                .iter()
                .flat_map(|known| {
                    known.environments.iter().map(|environment| {
                        result(environment).map(|result| (known.place.clone(), result))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        });
    }
    for (index, arm) in function.overload_arms.iter().enumerate() {
        collect_function_output_contracts(
            source,
            arm,
            declaration_span,
            ReferenceCallableBody::Overload(index),
            contracts,
        )?;
    }
    Ok(())
}

fn transfer_function_output_contracts(
    function: &mut ScalarFunction,
    contracts: &[ConcreteReferenceOutputContract],
    literal_contracts: &[LiteralOutputContract],
    call_exits: &[GlobalCallExitSnapshot],
    capture_input: &source_boundary::SourceCapturePreflightInput,
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for arm in &mut function.overload_arms {
        let arm_input = match source_boundary::preflight_input(&arm.reference_cfg.producer_owner, &capture_input.construction_forest, capture_input.substitutions.clone()) { Ok(input) => input, Err(error) => { errors.push(error); continue; } };
        errors.extend(transfer_function_output_contracts(
            arm,
            contracts,
            literal_contracts,
            call_exits,
            &arm_input,
            #[cfg(test)]
            trace,
        ));
    }
    if !function.reference_cfg.initial_points.is_empty() {
        if let Err(error) = source_boundary::validate_capture_preflight(&function.reference_cfg, capture_input) { errors.push(error); return errors; }
        function.reference_cfg.points = function.reference_cfg.initial_points.clone();
        errors.extend(transfer_reference_facts(
            &mut function.reference_cfg,
            &BTreeMap::new(),
            contracts,
            literal_contracts,
            call_exits,
            capture_input,
            #[cfg(test)]
            ReferenceTransferTestRole::Function,
            #[cfg(test)]
            trace,
        ));
    }
    errors
}

fn refresh_function_literal_cfgs(
    function: &mut ScalarFunction,
    contracts: &[ConcreteReferenceOutputContract],
    literal_contracts: &[LiteralOutputContract],
    capture_input: &source_boundary::SourceCapturePreflightInput,
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    let mut errors = refresh_literal_cfgs(
        &mut function.reference_cfg,
        contracts,
        literal_contracts,
        capture_input,
        #[cfg(test)]
        trace,
    );
    for arm in &mut function.overload_arms {
        let arm_input = match source_boundary::preflight_input(&arm.reference_cfg.producer_owner, &capture_input.construction_forest, capture_input.substitutions.clone()) { Ok(input) => input, Err(error) => { errors.push(error); continue; } };
        errors.extend(refresh_function_literal_cfgs(
            arm,
            contracts,
            literal_contracts,
            &arm_input,
            #[cfg(test)]
            trace,
        ));
    }
    errors
}

fn refresh_literal_cfgs(
    parent: &mut ScalarReferenceCfg,
    contracts: &[ConcreteReferenceOutputContract],
    literal_contracts: &[LiteralOutputContract],
    capture_input: &source_boundary::SourceCapturePreflightInput,
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    if let Err(error) = source_boundary::validate_capture_preflight(parent, capture_input) { return vec![error]; }
    let mut errors = Vec::new();
    for literal in &mut parent.literal_cfgs {
        let (capture_point, capture) = parent
            .initial_points
            .iter()
            .find_map(|point| match &point.kind {
                CfgPointKind::Capture { environment, .. }
                    if environment.literal == literal.literal =>
                {
                    Some((point.id, environment))
                }
                _ => None,
            })
            .expect("recorded literal capture point");
        let inherited = &parent.environments[capture.index];
        let child_input = match source_boundary::preflight_input(&literal.reference_cfg.producer_owner, &capture_input.construction_forest, capture_input.substitutions.clone()) {
            Ok(input) => input, Err(error) => { errors.push(error); continue; }
        };
        let cfg = &mut literal.reference_cfg;
        let mut origin_ids = Vec::new();
        for fact in &parent.facts {
            let index = match cfg.facts.iter().position(|existing| existing == fact) {
                Some(index) => index,
                None => {
                    let index = cfg.facts.len();
                    cfg.facts.push(fact.clone());
                    index
                }
            };
            origin_ids.push(ReferenceOriginId(index));
        }
        let environment_ids = parent
            .environments
            .iter()
            .enumerate()
            .map(|(parent_index, environment)| {
                let imported = parent
                    .environment_imports
                    .iter()
                    .find(|fact| fact.destination.index == parent_index);
                let index = imported
                    .and_then(|source| {
                        cfg.environment_imports
                            .iter()
                            .find(|known| {
                                known.table == source.table
                                    && known.source_index == source.source_index
                                    && known.source_instance == source.source_instance
                                    && known.destination_instance == source.destination_instance
                            })
                            .map(|known| known.destination.index)
                    })
                    .or_else(|| {
                        if imported.is_some() {
                            return None;
                        }
                        cfg.environments
                            .iter()
                            .enumerate()
                            .find(|(index, known)| {
                                !cfg.environment_imports
                                    .iter()
                                    .any(|fact| fact.destination.index == *index)
                                    && known.literal == environment.literal
                                    && known.instance == environment.instance
                            })
                            .map(|(index, _)| index)
                    })
                    .unwrap_or_else(|| {
                        let index = cfg.environments.len();
                        cfg.environments.push(environment.clone());
                        index
                    });
                CallableEnvironmentId {
                    literal: environment.literal.clone(),
                    index,
                }
            })
            .collect::<Vec<_>>();
        for imported in &parent.environment_imports {
            let mut rebased = imported.clone();
            if parent
                .environments
                .get(imported.destination.index)
                .is_none_or(|environment| {
                    environment.literal != imported.destination.literal
                        || environment.instance != imported.destination_instance
                })
            {
                errors.push(ReferenceAnalysisError::MissingContext {
                    source_span: literal.literal.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                });
                continue;
            }
            rebased.destination = environment_ids[imported.destination.index].clone();
            if cfg
                .environments
                .get(rebased.destination.index)
                .is_none_or(|environment| {
                    environment.literal != rebased.destination.literal
                        || environment.instance != rebased.destination_instance
                })
            {
                errors.push(ReferenceAnalysisError::MissingContext {
                    source_span: literal.literal.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                });
                continue;
            }
            if !cfg.environment_imports.contains(&rebased) {
                cfg.environment_imports.push(rebased);
            }
        }
        let rebase_failed = || ReferenceAnalysisError::MissingContext {
            source_span: literal.literal.clone(),
            reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
        };
        match rebase_mapped_capture_inputs(&inherited.inputs, &environment_ids, &origin_ids) {
            Some(inputs) => cfg.inputs = inputs,
            None => {
                errors.push(rebase_failed());
                cfg.inputs = Vec::new();
            }
        }
        let mut rebased_capture_paths = Vec::new();
        let mut paths_valid = true;
        for path in &inherited.capture_paths {
            match rebase_mapped_capture_inputs(path, &environment_ids, &origin_ids) {
                Some(rebased) => rebased_capture_paths.push(rebased),
                None => paths_valid = false,
            }
        }
        if !paths_valid {
            errors.push(rebase_failed());
        }
        cfg.capture_input_paths = rebased_capture_paths;
        let mut rebased_path_facts = Vec::new();
        for fact in parent
            .capture_input_path_facts
            .iter()
            .filter(|fact| fact.capture == capture_point)
        {
            let mut inputs = Vec::new();
            let mut valid = true;
            for input in &fact.inputs {
                let origin = match &input.origin {
                    CaptureInputOriginFact::Value => CaptureInputOriginFact::Value,
                    CaptureInputOriginFact::Callable { holders } => {
                        let mut rebased_holders = Vec::new();
                        for holder in holders {
                            let (Some(environment), Some(producer)) = (
                                environment_ids.get(holder.environment.index).cloned(),
                                rebase_holder_producer_ref(
                                    &holder.producer,
                                    &environment_ids,
                                    &origin_ids,
                                ),
                            ) else {
                                valid = false;
                                break;
                            };
                            rebased_holders.push(ConnectedCaptureHolderFact {
                                holder: holder.holder.clone(),
                                environment,
                                producer,
                            });
                        }
                        if !valid || rebased_holders.is_empty() {
                            valid = false;
                            break;
                        }
                        CaptureInputOriginFact::Callable {
                            holders: rebased_holders,
                        }
                    }
                };
                inputs.push(CaptureInputFact {
                    binding: input.binding.clone(),
                    origin,
                });
            }
            if !valid {
                errors.push(ReferenceAnalysisError::MissingContext {
                    source_span: literal.literal.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                });
                continue;
            }
            let rebased = CaptureInputPathFact {
                capture: fact.capture,
                capture_span: fact.capture_span.clone(),
                owner: fact.owner.clone(),
                inputs,
            };
            if !rebased_path_facts.contains(&rebased) {
                rebased_path_facts.push(rebased);
            }
        }
        cfg.capture_input_path_facts = rebased_path_facts;
        for (index, environment) in parent.environments.iter().enumerate() {
            let origins = rebase_mapped_origins(&environment.origins, &origin_ids);
            let loans = rebase_mapped_loans(&environment.loans, &origin_ids);
            let backings = environment
                .backings
                .iter()
                .map(|backing| rebase_mapped_backing(backing, &environment_ids, &origin_ids))
                .collect::<Option<Vec<_>>>();
            let inputs =
                rebase_mapped_capture_inputs(&environment.inputs, &environment_ids, &origin_ids);
            let mut capture_paths = Vec::new();
            let mut environment_valid = true;
            for path in &environment.capture_paths {
                match rebase_mapped_capture_inputs(path, &environment_ids, &origin_ids) {
                    Some(rebased) => capture_paths.push(rebased),
                    None => environment_valid = false,
                }
            }
            match (origins, loans, backings, inputs) {
                (Some(origins), Some(loans), Some(backings), Some(inputs)) if environment_valid => {
                    cfg.environments[environment_ids[index].index] = CallableEnvironmentFact {
                        literal: environment.literal.clone(),
                        instance: environment.instance.clone(),
                        origins,
                        owners: environment.owners.clone(),
                        loans,
                        backings,
                        inputs,
                        capture_paths,
                    };
                }
                _ => {
                    errors.push(rebase_failed());
                }
            }
        }
        cfg.points = cfg.initial_points.clone();
        errors.extend(transfer_reference_facts(
            cfg,
            &BTreeMap::new(),
            contracts,
            literal_contracts,
            &[],
            &child_input,
            #[cfg(test)]
            ReferenceTransferTestRole::LiteralRefresh,
            #[cfg(test)]
            trace,
        ));
        errors.extend(refresh_literal_cfgs(
            cfg,
            contracts,
            literal_contracts,
            &child_input,
            #[cfg(test)]
            trace,
        ));
    }
    errors
}

pub(super) fn resolve_reference_output_contracts(
    modules: &mut [ScalarModule],
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    let mut call_signatures = Vec::new();
    for module in modules.iter() {
        collect_literal_runtime_signatures(&module.top_level_reference_cfg, &mut call_signatures);
        for item in &module.items {
            if let ScalarItem::Function(function) = item {
                collect_literal_runtime_signatures(&function.reference_cfg, &mut call_signatures);
                record_runtime_call_signatures(
                    &module.source,
                    function,
                    function.name_span,
                    &mut call_signatures,
                );
            }
        }
    }
    for module in modules.iter_mut() {
        module.top_level_reference_cfg.call_signatures = call_signatures.clone();
        for item in &mut module.items {
            if let ScalarItem::Function(function) = item {
                install_runtime_call_signatures(function, &call_signatures);
            }
        }
    }
    let mut contracts = Vec::new();
    loop {
        let mut concrete = Vec::new();
        for module in modules.iter() {
            collect_cfg_concrete_output_contracts(
                &module.top_level_reference_cfg,
                &contracts,
                &mut concrete,
            );
            for item in &module.items {
                if let ScalarItem::Function(function) = item {
                    collect_concrete_output_contracts(function, &contracts, &mut concrete);
                }
            }
        }
        for module in modules.iter_mut() {
            let forest = match source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete) { Ok(forest) => forest, Err(error) => return vec![error] };
            for item in &mut module.items {
                if let ScalarItem::Function(function) = item {
                    let capture_input = match source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()) { Ok(input) => input, Err(error) => return vec![error] };
                    let _ = transfer_function_output_contracts(
                        function,
                        &concrete,
                        &[],
                        &[],
                        &capture_input,
                        #[cfg(test)]
                        trace,
                    );
                }
            }
        }
        let mut next = Vec::new();
        for module in modules.iter() {
            for item in &module.items {
                if let ScalarItem::Function(function) = item {
                    if let Err(error) = collect_function_output_contracts(
                        &module.source,
                        function,
                        function.name_span,
                        ReferenceCallableBody::Declaration,
                        &mut next,
                    ) {
                        return vec![error];
                    }
                }
            }
        }
        if next == contracts {
            let mut literal_contracts = match collect_project_literal_output_contracts(modules) {
                Ok(contracts) => contracts,
                Err(error) => return vec![error],
            };
            loop {
                let mut literal_errors = Vec::new();
                let call_exit_snapshots = collect_global_call_exit_snapshots(modules);
                for module in modules.iter_mut() {
                    let forest = match source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete) { Ok(forest) => forest, Err(error) => return vec![error] };
                    let module_input = match source_boundary::preflight_input(&module.top_level_reference_cfg.producer_owner, &forest, BTreeMap::new()) { Ok(input) => input, Err(error) => return vec![error] };
                    for item in &mut module.items {
                        if let ScalarItem::Function(function) = item {
                            let capture_input = match source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()) { Ok(input) => input, Err(error) => return vec![error] };
                            literal_errors.extend(transfer_function_output_contracts(
                                function,
                                &concrete,
                                &literal_contracts,
                                &call_exit_snapshots,
                                &capture_input,
                                #[cfg(test)]
                                trace,
                            ));
                        }
                    }
                    literal_errors.extend(refresh_literal_cfgs(
                        &mut module.top_level_reference_cfg,
                        &concrete,
                        &literal_contracts,
                        &module_input,
                        #[cfg(test)]
                        trace,
                    ));
                    for item in &mut module.items {
                        if let ScalarItem::Function(function) = item {
                            let capture_input = match source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()) { Ok(input) => input, Err(error) => return vec![error] };
                            literal_errors.extend(refresh_function_literal_cfgs(
                                function,
                                &concrete,
                                &literal_contracts,
                                &capture_input,
                                #[cfg(test)]
                                trace,
                            ));
                        }
                    }
                }
                let updated = match collect_project_literal_output_contracts(modules) {
                    Ok(contracts) => contracts,
                    Err(error) => return vec![error],
                };
                if updated == literal_contracts {
                    literal_errors.extend(resolve_runtime_backing_inputs(
                        modules,
                        &concrete,
                        &literal_contracts,
                        #[cfg(test)]
                        trace,
                    ));
                    return literal_errors;
                }
                literal_contracts = updated;
            }
        }
        contracts = next;
    }
}

fn collect_literal_runtime_signatures(
    cfg: &ScalarReferenceCfg,
    signatures: &mut Vec<RuntimeCallSignature>,
) {
    for point in &cfg.initial_points {
        let CfgPointKind::Capture {
            environment,
            signature:
                ScalarType::Callable {
                    parameters,
                    outputs,
                },
            parameter_spans,
            body,
            ..
        } = &point.kind
        else {
            continue;
        };
        signatures.push(RuntimeCallSignature {
            source: environment.literal.source.clone(),
            declaration_span: environment.literal.range,
            selection: RuntimeCallSelection::Declaration,
            parameters: parameters.clone(),
            outputs: outputs
                .outputs
                .iter()
                .map(|output| output.ty.clone())
                .collect(),
            generic_parameters: Vec::new(),
            bindings: parameter_spans
                .iter()
                .map(|span| ReferenceBindingId {
                    source: environment.literal.source.clone(),
                    function_span: environment.literal.range,
                    declaration_span: *span,
                    block_span: body.span,
                    kind: ReferenceBindingKind::Declared,
                })
                .collect(),
        });
    }
    for literal in &cfg.literal_cfgs {
        collect_literal_runtime_signatures(&literal.reference_cfg, signatures);
    }
}

fn collect_runtime_call_paths(
    cfg: &ScalarReferenceCfg,
    paths: &mut Vec<Vec<RuntimeCallArgumentFact>>,
) {
    for point in &cfg.points {
        if matches!(point.kind, CfgPointKind::Call { output: 0, .. }) {
            for path in &point.backing_argument_paths {
                let facts = path.clone();
                if !paths.contains(&facts) {
                    paths.push(facts);
                }
            }
        }
    }
    for literal in &cfg.literal_cfgs {
        collect_runtime_call_paths(&literal.reference_cfg, paths);
    }
}

fn collect_runtime_output_contracts(
    source: &SourceIdentity,
    function: &ScalarFunction,
    declaration_span: ByteSpan,
    selection: RuntimeCallSelection,
    contracts: &mut Vec<RuntimeBackingOutputContract>,
) {
    let ScalarType::Callable { outputs, .. } = &function.signature else {
        return;
    };
    for point in &function.reference_cfg.points {
        let CfgPointKind::ReturnOutput {
            output,
            value_point,
            ref backing_action,
            ..
        } = point.kind
        else {
            continue;
        };
        let direct = matches!(outputs.outputs[output].ty, ScalarType::RuntimeArray { .. });
        if !direct && point.possible_projected_backings.is_empty() {
            continue;
        }
        let mut sources = Vec::new();
        if direct {
            if let CfgPointKind::Read { binding } =
                &function.reference_cfg.initial_points[value_point.0].kind
            {
                if let Some(index) = function
                    .parameter_spans
                    .iter()
                    .position(|span| *span == binding.declaration_span)
                {
                    sources.push(RuntimeOutputBackingSource::Parameter(index));
                }
            }
            for fact in &point.possible_backings {
                let candidate = if let Some(index) = function
                    .reference_cfg
                    .backing_input_paths
                    .iter()
                    .flat_map(|path| path.iter())
                    .find(|input| input.value == fact.value)
                    .and_then(|input| {
                        function
                            .parameter_spans
                            .iter()
                            .position(|span| *span == input.owner.declaration_span)
                    }) {
                    RuntimeOutputBackingSource::Parameter(index)
                } else if let RuntimeValuePosition::Parameter { declaration_span } =
                    fact.value.position
                {
                    let index = function
                        .parameter_spans
                        .iter()
                        .position(|span| *span == declaration_span)
                        .expect("typed parameter backing");
                    RuntimeOutputBackingSource::Parameter(index)
                } else {
                    let action = match backing_action {
                        RuntimeReturnBackingAction::CopyFixed { .. } => {
                            RuntimeOutputBackingAction::CopyFixed
                        }
                        RuntimeReturnBackingAction::MoveFixed { .. } => {
                            RuntimeOutputBackingAction::MoveFixed
                        }
                        RuntimeReturnBackingAction::Transfer => {
                            RuntimeOutputBackingAction::Allocate
                        }
                    };
                    RuntimeOutputBackingSource::Created {
                        origin: fact.origin.clone(),
                        action,
                        lineage: fact.value.clone(),
                        length: fact.length.clone(),
                        callable_family: fact.callable_family.clone(),
                        owner: fact.owner.clone(),
                    }
                };
                if !sources.contains(&candidate) {
                    sources.push(candidate);
                }
            }
        }
        if direct {
            let contract = RuntimeBackingOutputContract {
                source: source.clone(),
                declaration_span,
                selection: selection.clone(),
                output,
                projections: Vec::new(),
                sources,
            };
            if !contracts.contains(&contract) {
                contracts.push(contract);
            }
        }
        for projected in &point.possible_projected_backings {
            let fact = &projected.backing;
            let parameter = function
                .reference_cfg
                .projected_backing_input_paths
                .iter()
                .flat_map(|path| path.iter())
                .find(|input| {
                    input.projections == projected.place.projections
                        && input.backing.value == fact.value
                })
                .map(|input| input.position)
                .or_else(|| match fact.value.position {
                    RuntimeValuePosition::Parameter { declaration_span } => function
                        .parameter_spans
                        .iter()
                        .position(|span| *span == declaration_span),
                    _ => None,
                });
            let candidate = if let Some(position) = parameter {
                RuntimeOutputBackingSource::Parameter(position)
            } else {
                RuntimeOutputBackingSource::Created {
                    origin: fact.origin.clone(),
                    action: match backing_action {
                        RuntimeReturnBackingAction::CopyFixed { .. } => {
                            RuntimeOutputBackingAction::CopyFixed
                        }
                        RuntimeReturnBackingAction::MoveFixed { .. } => {
                            RuntimeOutputBackingAction::MoveFixed
                        }
                        RuntimeReturnBackingAction::Transfer => {
                            RuntimeOutputBackingAction::Allocate
                        }
                    },
                    lineage: fact.value.clone(),
                    length: fact.length.clone(),
                    callable_family: fact.callable_family.clone(),
                    owner: fact.owner.clone(),
                }
            };
            if let Some(contract) = contracts.iter_mut().find(|contract| {
                contract.source == *source
                    && contract.declaration_span == declaration_span
                    && contract.selection == selection
                    && contract.output == output
                    && contract.projections == projected.place.projections
            }) {
                if !contract.sources.contains(&candidate) {
                    contract.sources.push(candidate);
                }
            } else {
                contracts.push(RuntimeBackingOutputContract {
                    source: source.clone(),
                    declaration_span,
                    selection: selection.clone(),
                    output,
                    projections: projected.place.projections.clone(),
                    sources: vec![candidate],
                });
            }
        }
    }
    for (index, arm) in function.overload_arms.iter().enumerate() {
        collect_runtime_output_contracts(
            source,
            arm,
            declaration_span,
            RuntimeCallSelection::Overload(index),
            contracts,
        );
    }
}

fn install_runtime_output_contracts(
    cfg: &mut ScalarReferenceCfg,
    contracts: &[RuntimeBackingOutputContract],
) -> bool {
    let changed = cfg.backing_output_contracts != contracts;
    cfg.backing_output_contracts = contracts.to_vec();
    for literal in &mut cfg.literal_cfgs {
        install_runtime_output_contracts(&mut literal.reference_cfg, contracts);
    }
    changed
}

fn install_function_runtime_output_contracts(
    function: &mut ScalarFunction,
    contracts: &[RuntimeBackingOutputContract],
) -> bool {
    let mut changed = install_runtime_output_contracts(&mut function.reference_cfg, contracts);
    for arm in &mut function.overload_arms {
        changed |= install_function_runtime_output_contracts(arm, contracts);
    }
    changed
}

fn unresolved_runtime_outputs(
    cfg: &ScalarReferenceCfg,
    literal_contracts: &[LiteralOutputContract],
    errors: &mut Vec<ReferenceAnalysisError>,
) {
    let mut reachable = BTreeSet::from([CfgPointId(0)]);
    let mut pending = vec![CfgPointId(0)];
    while let Some(id) = pending.pop() {
        for successor in &cfg.points[id.0].successors {
            if reachable.insert(*successor) {
                pending.push(*successor);
            }
        }
    }
    for point in &cfg.points {
        if !reachable.contains(&point.id) {
            continue;
        }
        let CfgPointKind::Call {
            target,
            output,
            argument_points,
            ..
        } = &point.kind
        else {
            continue;
        };
        if let ResolvedCallTarget::ModuleCallable {
            source,
            declaration_span,
            concrete,
        } = target
        {
            let selection = match concrete {
                ConcreteCallSelection::Declaration => RuntimeCallSelection::Declaration,
                ConcreteCallSelection::Overload(arm) => {
                    RuntimeCallSelection::Overload(arm.arm_index)
                }
            };
            for contract in cfg.backing_output_contracts.iter().filter(|contract| {
                contract.source == *source
                    && contract.declaration_span == *declaration_span
                    && contract.selection == selection
                    && contract.output == *output
                    && !contract.projections.is_empty()
            }) {
                let arguments = cfg
                    .points
                    .iter()
                    .find(|call| {
                        call.source_span == point.source_span
                            && matches!(call.kind, CfgPointKind::Call { output: 0, .. })
                    })
                    .expect("evaluated multi-output call");
                let missing_input = contract.sources.iter().any(|candidate| match candidate {
                    RuntimeOutputBackingSource::Parameter(position) => {
                        !arguments.backing_argument_paths.iter().any(|path| {
                            path.iter().any(|input| {
                                input.position == *position
                                    && input.projections == contract.projections
                            })
                        })
                    }
                    RuntimeOutputBackingSource::Created { .. } => false,
                });
                if missing_input
                    || !point
                        .possible_projected_backings
                        .iter()
                        .any(|fact| fact.place.projections == contract.projections)
                {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    });
                }
            }
        }
        let templates = match target {
            ResolvedCallTarget::ModuleCallable {
                source,
                declaration_span,
                concrete,
            } => {
                let selection = match concrete {
                    ConcreteCallSelection::Declaration => RuntimeCallSelection::Declaration,
                    ConcreteCallSelection::Overload(arm) => {
                        RuntimeCallSelection::Overload(arm.arm_index)
                    }
                };
                cfg.backing_output_contracts
                    .iter()
                    .filter(|contract| {
                        contract.source == *source
                            && contract.declaration_span == *declaration_span
                            && contract.selection == selection
                            && contract.output == *output
                    })
                    .flat_map(|contract| contract.sources.iter())
                    .collect::<Vec<_>>()
            }
            ResolvedCallTarget::CallableField { .. } => point
                .possible_callee_environments
                .iter()
                .flat_map(|environment| {
                    literal_contracts.iter().filter(move |contract| {
                        contract.literal == environment.literal
                            && contract.output == *output
                            && contract.runtime_array
                    })
                })
                .flat_map(|contract| contract.backings.iter())
                .collect::<Vec<_>>(),
            _ => continue,
        };
        let missing_forwarded_owner = templates.iter().any(|candidate| {
            let RuntimeOutputBackingSource::Parameter(position) = candidate else { return false };
            let symbolic = argument_points.get(*position).is_some_and(|id| {
                matches!(&cfg.initial_points[id.0].kind, CfgPointKind::Read { binding } if cfg.backing_parameters.contains(binding))
            });
            !symbolic && !point.backing_argument_paths.iter().any(|path| path.iter().any(|input| input.position == *position))
        });
        let symbolic_parameter = !templates.is_empty() && templates.iter().all(|candidate| match candidate {
            RuntimeOutputBackingSource::Parameter(position) => argument_points.get(*position).is_some_and(|id| {
                matches!(&cfg.initial_points[id.0].kind, CfgPointKind::Read { binding } if cfg.backing_parameters.contains(binding))
            }),
            RuntimeOutputBackingSource::Created { .. } => false,
        });
        let runtime_output = match target {
            ResolvedCallTarget::ModuleCallable {
                source,
                declaration_span,
                concrete,
            } => cfg.call_signatures.iter().any(|signature| {
                signature.source == *source
                    && signature.declaration_span == *declaration_span
                    && match (&signature.selection, concrete) {
                        (RuntimeCallSelection::Declaration, ConcreteCallSelection::Declaration) => {
                            true
                        }
                        (
                            RuntimeCallSelection::Overload(index),
                            ConcreteCallSelection::Overload(arm),
                        ) => *index == arm.arm_index,
                        _ => false,
                    }
                    && matches!(
                        signature.outputs.get(*output),
                        Some(ScalarType::RuntimeArray { .. })
                    )
            }),
            ResolvedCallTarget::CallableField { .. } => {
                !point.possible_callee_environments.is_empty()
                    && point
                        .possible_callee_environments
                        .iter()
                        .any(|environment| {
                            literal_contracts.iter().any(|contract| {
                                contract.literal == environment.literal
                                    && contract.output == *output
                                    && contract.runtime_array
                            })
                        })
            }
            _ => false,
        };
        if runtime_output
            && (missing_forwarded_owner
                || (!symbolic_parameter
                    && !point.possible_backings.iter().any(|fact| {
                        fact.owner.kind
                            == ReferenceBindingKind::CallOutput {
                                point: point.id,
                                output: *output,
                            }
                    })))
        {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: point.source_span.clone(),
                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
            });
        }
    }
    for literal in &cfg.literal_cfgs {
        unresolved_runtime_outputs(&literal.reference_cfg, literal_contracts, errors);
    }
}

fn unresolved_function_runtime_outputs(
    function: &ScalarFunction,
    literal_contracts: &[LiteralOutputContract],
    errors: &mut Vec<ReferenceAnalysisError>,
) {
    unresolved_runtime_outputs(&function.reference_cfg, literal_contracts, errors);
    for arm in &function.overload_arms {
        unresolved_function_runtime_outputs(arm, literal_contracts, errors);
    }
}

fn update_runtime_input_paths(
    cfg: &mut ScalarReferenceCfg,
    source: &SourceIdentity,
    function_span: ByteSpan,
    paths: &[Vec<RuntimeCallArgumentFact>],
) -> bool {
    let incoming = paths
        .iter()
        .filter(|path| {
            path.iter().all(|fact| {
                fact.backing.owner.source == *source
                    && fact.backing.owner.function_span == function_span
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    let direct = incoming
        .iter()
        .map(|path| {
            path.iter()
                .filter(|input| input.projections.is_empty())
                .map(|input| input.backing.clone())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let projected = if incoming
        .iter()
        .any(|path| path.iter().any(|input| !input.projections.is_empty()))
    {
        incoming
    } else {
        Vec::new()
    };
    let changed = (!cfg.backing_parameters.is_empty() && cfg.backing_input_paths != direct)
        || cfg.projected_backing_input_paths != projected;
    if changed {
        if !cfg.backing_parameters.is_empty() {
            cfg.backing_input_paths = direct;
        }
        cfg.projected_backing_input_paths = projected;
    }
    changed
}

fn distribute_runtime_input_paths(
    function: &mut ScalarFunction,
    source: &SourceIdentity,
    paths: &[Vec<RuntimeCallArgumentFact>],
) -> bool {
    let mut changed =
        update_runtime_input_paths(&mut function.reference_cfg, source, function.span, paths);
    changed |= distribute_literal_input_paths(&mut function.reference_cfg, paths);
    for arm in &mut function.overload_arms {
        changed |= distribute_runtime_input_paths(arm, source, paths);
    }
    changed
}

fn distribute_literal_input_paths(
    cfg: &mut ScalarReferenceCfg,
    paths: &[Vec<RuntimeCallArgumentFact>],
) -> bool {
    let mut changed = false;
    for literal in &mut cfg.literal_cfgs {
        changed |= update_runtime_input_paths(
            &mut literal.reference_cfg,
            &literal.literal.source,
            literal.literal.range,
            paths,
        );
        changed |= distribute_literal_input_paths(&mut literal.reference_cfg, paths);
    }
    changed
}

#[derive(Clone)]
struct GlobalCallableInputProducer {
    path: GlobalCallableCallInputFact,
    facts: Vec<ReferenceOrigin>,
    environments: Vec<CallableEnvironmentFact>,
}

fn collect_global_callable_inputs(
    cfg: &ScalarReferenceCfg,
    calls: &mut Vec<GlobalCallableInputProducer>,
) {
    for point in &cfg.points {
        let CfgPointKind::Call {
            target, output: 0, ..
        } = &point.kind
        else {
            continue;
        };
        if !matches!(target, ResolvedCallTarget::ModuleCallable { .. }) {
            continue;
        }
        for path in &point.global_callable_call_paths {
            let fact = GlobalCallableCallInputFact {
                occurrence: GlobalCallOccurrenceKey {
                    caller: point.source_span.source.clone(),
                    caller_cfg: cfg.producer_owner.clone(),
                    point: point.id,
                    span: point.source_span.clone(),
                    callee: target.clone(),
                    output: 0,
                },
                call: point.source_span.clone(),
                target: target.clone(),
                holders: path.clone(),
                producer: cfg.points[0].source_span.clone(),
                environment_indices: Vec::new(),
            };
            if !calls.iter().any(|known| known.path == fact) {
                calls.push(GlobalCallableInputProducer {
                    path: fact,
                    facts: cfg.facts.clone(),
                    environments: cfg.environments.clone(),
                });
            }
        }
    }
}

fn rebase_global_callable_input(
    cfg: &mut ScalarReferenceCfg,
    producer: &GlobalCallableInputProducer,
) -> Result<GlobalCallableCallInputFact, ReferenceAnalysisError> {
    let path = &producer.path;
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: path.call.clone(),
        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
    };
    if path.producer.source != path.call.source
        || producer
            .environments
            .iter()
            .enumerate()
            .any(|(index, environment)| {
                let valid = |id: &CallableEnvironmentId| {
                    producer
                        .environments
                        .get(id.index)
                        .is_some_and(|fact| fact.literal == id.literal)
                };
                environment
                    .origins
                    .iter()
                    .any(|origin| origin.0 >= producer.facts.len())
                    || environment
                        .loans
                        .iter()
                        .any(|loan| loan.origin.0 >= producer.facts.len())
                    || environment
                        .inputs
                        .iter()
                        .chain(environment.capture_paths.iter().flatten())
                        .any(|input| {
                            input
                                .origins
                                .iter()
                                .any(|origin| origin.0 >= producer.facts.len())
                                || input.environments.iter().any(|id| !valid(id))
                                || input.projected.iter().any(|projection| {
                                    projection.environments.iter().any(|id| !valid(id))
                                })
                        })
                    || producer.environments.get(index).is_none()
            })
        || path.holders.iter().any(|holder| {
            holder
                .origins
                .iter()
                .any(|origin| origin.0 >= producer.facts.len())
                || holder.relations.iter().any(|relation| {
                    !producer
                        .environments
                        .get(relation.environment.index)
                        .is_some_and(|fact| fact.literal == relation.environment.literal)
                        || relation.capture_paths.iter().flatten().any(|input| {
                            input
                                .origins
                                .iter()
                                .any(|origin| origin.0 >= producer.facts.len())
                                || input.environments.iter().any(|id| {
                                    !producer
                                        .environments
                                        .get(id.index)
                                        .is_some_and(|fact| fact.literal == id.literal)
                                })
                                || input.projected.iter().any(|projection| {
                                    projection.environments.iter().any(|id| {
                                        !producer
                                            .environments
                                            .get(id.index)
                                            .is_some_and(|fact| fact.literal == id.literal)
                                    })
                                })
                        })
                })
        })
    {
        return Err(invalid());
    }
    let previous = cfg
        .global_callable_input_paths
        .iter()
        .filter(|known| {
            known.call == path.call
                && known.target == path.target
                && known.producer == path.producer
        })
        .collect::<Vec<_>>();
    if previous
        .iter()
        .skip(1)
        .any(|known| known.environment_indices != previous[0].environment_indices)
    {
        return Err(invalid());
    }
    // Cut 3: self-distribution (a recursive entry whose producer snapshot is
    // the callee table itself) reuses the shared table by identity instead of
    // duplicating it positionally. A recursive invocation therefore allocates
    // no new abstract environment IDs for its own entries.
    let shared = producer.environments == cfg.environments && producer.facts == cfg.facts;
    let old_indices = previous
        .first()
        .map(|known| known.environment_indices.as_slice())
        .unwrap_or(&[]);
    if !shared
        && old_indices.iter().any(|id| {
            !cfg.environments
                .get(id.index)
                .is_some_and(|fact| fact.literal == id.literal)
        })
    {
        return Err(invalid());
    }
    let (indices, origins) = if shared {
        (
            cfg.environments
                .iter()
                .enumerate()
                .map(|(index, fact)| CallableEnvironmentId {
                    literal: fact.literal.clone(),
                    index,
                })
                .collect::<Vec<_>>(),
            (0..cfg.facts.len())
                .map(ReferenceOriginId)
                .collect::<Vec<_>>(),
        )
    } else {
        let mut indices = old_indices.to_vec();
        for environment in producer.environments.iter().skip(indices.len()) {
            let index = cfg.environments.len();
            cfg.environments.push(environment.clone());
            indices.push(CallableEnvironmentId {
                literal: environment.literal.clone(),
                index,
            });
        }
        let origins = producer
            .facts
            .iter()
            .map(|origin| {
                let index = cfg
                    .facts
                    .iter()
                    .position(|known| known == origin)
                    .unwrap_or_else(|| {
                        let index = cfg.facts.len();
                        cfg.facts.push(origin.clone());
                        index
                    });
                ReferenceOriginId(index)
            })
            .collect::<Vec<_>>();
        (indices, origins)
    };
    let rebase_inputs = |inputs: &[CallableCaptureInput]| {
        rebase_mapped_capture_inputs(inputs, &indices, &origins).ok_or_else(invalid)
    };
    for (index, environment) in producer.environments.iter().enumerate() {
        let rebased_origins =
            rebase_mapped_origins(&environment.origins, &origins).ok_or_else(invalid)?;
        let rebased_loans =
            rebase_mapped_loans(&environment.loans, &origins).ok_or_else(invalid)?;
        let rebased_backings = environment
            .backings
            .iter()
            .map(|backing| rebase_mapped_backing(backing, &indices, &origins))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(invalid)?;
        let rebased_inputs = rebase_inputs(&environment.inputs)?;
        let mut rebased_paths = Vec::new();
        for path in &environment.capture_paths {
            rebased_paths.push(rebase_inputs(path)?);
        }
        cfg.environments[indices[index].index] = CallableEnvironmentFact {
            literal: environment.literal.clone(),
            instance: environment.instance.clone(),
            origins: rebased_origins,
            owners: environment.owners.clone(),
            loans: rebased_loans,
            backings: rebased_backings,
            inputs: rebased_inputs,
            capture_paths: rebased_paths,
        };
    }
    let mut rebased = path.clone();
    rebased.environment_indices = indices.clone();
    for holder in &mut rebased.holders {
        holder.origins = holder
            .origins
            .iter()
            .map(|origin| origins.get(origin.0).copied().ok_or_else(invalid))
            .collect::<Result<BTreeSet<_>, _>>()?;
        for relation in &mut holder.relations {
            relation.environment = indices
                .get(relation.environment.index)
                .cloned()
                .ok_or_else(invalid)?;
            let mut rebased_paths = Vec::new();
            for path in &relation.capture_paths {
                rebased_paths.push(rebase_inputs(path)?);
            }
            relation.capture_paths = rebased_paths;
        }
        holder.backings = holder
            .backings
            .iter()
            .map(|backing| rebase_mapped_backing(backing, &indices, &origins))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(invalid)?;
        holder.projected_backings = holder
            .projected_backings
            .iter()
            .map(|projected| rebase_mapped_projected_backing(projected, &indices, &origins))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(invalid)?;
    }
    Ok(rebased)
}

fn distribute_global_callable_inputs(
    source: &SourceIdentity,
    function: &mut ScalarFunction,
    calls: &[GlobalCallableInputProducer],
) -> Result<bool, ReferenceAnalysisError> {
    let selected = calls
        .iter()
        .filter(|call| match &call.path.target {
            ResolvedCallTarget::ModuleCallable {
                source: declaring,
                declaration_span,
                concrete: ConcreteCallSelection::Declaration,
            } => declaring == source && *declaration_span == function.name_span,
            _ => false,
        })
        .collect::<Vec<_>>();
    let old_incoming = function.reference_cfg.global_callable_input_paths.clone();
    let mut incoming = Vec::new();
    for call in selected {
        let rebased = rebase_global_callable_input(&mut function.reference_cfg, call)?;
        if !function
            .reference_cfg
            .global_callable_input_paths
            .iter()
            .any(|known| {
                known.call == rebased.call
                    && known.target == rebased.target
                    && known.producer == rebased.producer
            })
        {
            function
                .reference_cfg
                .global_callable_input_paths
                .push(rebased.clone());
        }
        incoming.push(rebased);
    }
    let mut changed = old_incoming != incoming;
    if changed {
        function.reference_cfg.global_callable_input_paths = incoming;
    }
    for (index, arm) in function.overload_arms.iter_mut().enumerate() {
        let selected = calls
            .iter()
            .filter(|call| match &call.path.target {
                ResolvedCallTarget::ModuleCallable {
                    source: declaring,
                    declaration_span,
                    concrete: ConcreteCallSelection::Overload(selection),
                } if declaring == source
                    && *declaration_span == function.name_span
                    && selection.arm_index == index =>
                {
                    true
                }
                _ => false,
            })
            .collect::<Vec<_>>();
        let old_arm_paths = arm.reference_cfg.global_callable_input_paths.clone();
        let mut arm_paths = Vec::new();
        for call in selected {
            let rebased = rebase_global_callable_input(&mut arm.reference_cfg, call)?;
            if !arm
                .reference_cfg
                .global_callable_input_paths
                .iter()
                .any(|known| {
                    known.call == rebased.call
                        && known.target == rebased.target
                        && known.producer == rebased.producer
                })
            {
                arm.reference_cfg
                    .global_callable_input_paths
                    .push(rebased.clone());
            }
            arm_paths.push(rebased);
        }
        if old_arm_paths != arm_paths {
            arm.reference_cfg.global_callable_input_paths = arm_paths;
            changed = true;
        }
    }
    Ok(changed)
}

// Cut 3: same-occurrence call-entry/exit effect fixed point.
//
// A call occurrence is keyed by caller identity, caller CFG, CFG point,
// source span, resolved callee and projected output, with its correlated
// whole input alternative. Each callee exit fact carries its entry-path key,
// the reachable exit point, the whole post-call global holder state, ordered
// write kills/installs and connected actions. Facts apply only at their
// originating caller occurrence and successor edge; nested calls push results
// into the parent continuation before the parent exits. Per-write output
// effects stay members of their path, never independently merged summaries.
// Runtime actions emit once where the write executes inside the callee's
// protected mutation; the caller application moves ownership state
// (relations, callable maps, provenance, backings, origins) so the old
// holder drop and the new holder install travel together without a second
// retirement.

#[derive(Clone, Debug)]
pub(super) struct GlobalCallExitSnapshot {
    pub target: ResolvedCallTarget,
    pub owner: ProducerCfgOwnerId,
    pub environments: Vec<CallableEnvironmentFact>,
    pub facts: Vec<ReferenceOrigin>,
    pub imports: Vec<GlobalCallExitImportFact>,
    pub exits: Vec<GlobalCallableCallExitFact>,
}

fn global_exit_holder_binding(holder: &CallableHolderPlace) -> Option<ReferenceBindingId> {
    match holder {
        CallableHolderPlace::Binding(binding) => Some(binding.clone()),
        CallableHolderPlace::Projection(place) => Some(place.binding.clone()),
        _ => None,
    }
}

fn global_exit_binding_is_global(binding: &ReferenceBindingId) -> bool {
    binding.function_span == ByteSpan::new(0, 0)
        && matches!(binding.kind, ReferenceBindingKind::Declared)
}

fn same_global_holder_set(
    left: &[GlobalCallableCallPathFact],
    right: &[GlobalCallableCallPathFact],
) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .all(|fact| right.iter().any(|known| known == fact))
}

fn collect_global_call_exit_snapshots(modules: &[ScalarModule]) -> Vec<GlobalCallExitSnapshot> {
    let mut snapshots = Vec::new();
    for module in modules {
        for item in &module.items {
            let ScalarItem::Function(function) = item else {
                continue;
            };
            collect_function_exit_snapshots(&module.source, function, &mut snapshots);
        }
    }
    snapshots
}

fn collect_function_exit_snapshots(
    source: &SourceIdentity,
    function: &ScalarFunction,
    snapshots: &mut Vec<GlobalCallExitSnapshot>,
) {
    snapshots.push(GlobalCallExitSnapshot {
        target: ResolvedCallTarget::ModuleCallable {
            source: source.clone(),
            declaration_span: function.name_span,
            concrete: ConcreteCallSelection::Declaration,
        },
        owner: function.reference_cfg.producer_owner.clone(),
        environments: function.reference_cfg.environments.clone(),
        facts: function.reference_cfg.facts.clone(),
        imports: function.reference_cfg.global_call_exit_imports.clone(),
        exits: function.reference_cfg.global_call_exit_facts.clone(),
    });
    for (index, arm) in function.overload_arms.iter().enumerate() {
        snapshots.push(GlobalCallExitSnapshot {
            target: ResolvedCallTarget::ModuleCallable {
                source: source.clone(),
                declaration_span: function.name_span,
                concrete: ConcreteCallSelection::Overload(ScalarOverloadSelection {
                    arm_index: index,
                    substitutions: BTreeMap::new(),
                }),
            },
            owner: arm.reference_cfg.producer_owner.clone(),
            environments: arm.reference_cfg.environments.clone(),
            facts: arm.reference_cfg.facts.clone(),
            imports: arm.reference_cfg.global_call_exit_imports.clone(),
            exits: arm.reference_cfg.global_call_exit_facts.clone(),
        });
    }
}

fn refresh_function_call_exit_facts(
    function: &mut ScalarFunction,
    errors: &mut Vec<ReferenceAnalysisError>,
) -> bool {
    let mut changed = refresh_cfg_call_exit_facts(&mut function.reference_cfg, errors);
    for arm in &mut function.overload_arms {
        changed |= refresh_cfg_call_exit_facts(&mut arm.reference_cfg, errors);
    }
    changed
}

fn refresh_cfg_call_exit_facts(
    cfg: &mut ScalarReferenceCfg,
    errors: &mut Vec<ReferenceAnalysisError>,
) -> bool {
    let mut writes = Vec::new();
    for point in &cfg.points {
        for effect in &point.global_callable_output_effects {
            if !writes.contains(effect) {
                writes.push(effect.clone());
            }
        }
    }
    // Group inputs by equal resolved callee and whole correlated holders so a
    // recursive re-entry of an equal pair reuses its summary without new IDs.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (index, input) in cfg.global_callable_input_paths.iter().enumerate() {
        let mut placed = false;
        for group in &mut groups {
            let first = &cfg.global_callable_input_paths[group[0]];
            if first.target == input.target
                && same_global_holder_set(&first.holders, &input.holders)
            {
                group.push(index);
                placed = true;
                break;
            }
        }
        if !placed {
            groups.push(vec![index]);
        }
    }
    let mut fresh = Vec::new();
    for group in &groups {
        let template_input = cfg.global_callable_input_paths[group[0]].clone();
        let template = build_global_call_exit_facts(cfg, &template_input, &writes, errors);
        for index in group {
            if *index == group[0] {
                fresh.extend(template.clone());
            } else {
                for fact in &template {
                    let mut reused = fact.clone();
                    reused.entry = cfg.global_callable_input_paths[*index].occurrence.clone();
                    if !fresh.contains(&reused) {
                        fresh.push(reused);
                    }
                }
            }
        }
    }
    if fresh != cfg.global_call_exit_facts {
        cfg.global_call_exit_facts = fresh;
        true
    } else {
        false
    }
}

fn validate_exit_input_provenance(
    cfg: &ScalarReferenceCfg,
    input: &GlobalCallableCallInputFact,
) -> bool {
    for holder_fact in &input.holders {
        for origin in &holder_fact.origins {
            if cfg.facts.get(origin.0).is_none() {
                return false;
            }
        }
        for relation in &holder_fact.relations {
            let Some(fact) = cfg.environments.get(relation.environment.index) else {
                return false;
            };
            if fact.literal != relation.environment.literal {
                return false;
            }
            for alternatives in &relation.capture_paths {
                for capture in alternatives {
                    for id in capture.environments.iter().chain(
                        capture
                            .projected
                            .iter()
                            .flat_map(|known| &known.environments),
                    ) {
                        let Some(fact) = cfg.environments.get(id.index) else {
                            return false;
                        };
                        if fact.literal != id.literal {
                            return false;
                        }
                    }
                    for origin in &capture.origins {
                        if cfg.facts.get(origin.0).is_none() {
                            return false;
                        }
                    }
                }
            }
        }
    }
    true
}

fn exit_selected_action_binding(action: &CallableHolderAction) -> Option<ReferenceBindingId> {
    let place = match action {
        CallableHolderAction::Drop { holder, .. } => Some(holder.clone()),
        CallableHolderAction::Retain { to, .. }
        | CallableHolderAction::Move { to, .. }
        | CallableHolderAction::ReturnTransfer { to, .. } => Some(to.clone()),
        CallableHolderAction::RetainSelected { to, .. } => {
            Some(CallableHolderPlace::Projection(to.place.clone()))
        }
        CallableHolderAction::MoveSelected { to, .. } => {
            Some(CallableHolderPlace::Projection(to.place.clone()))
        }
        CallableHolderAction::RetainCheckedSelected { to, .. } => Some(to.clone()),
        CallableHolderAction::DropSelected { holder, .. }
        | CallableHolderAction::SkipUninitializedSelected { holder, .. } => {
            Some(CallableHolderPlace::Projection(holder.place.clone()))
        }
        CallableHolderAction::DropArrayElements { backing, .. } => {
            Some(CallableHolderPlace::Binding(backing.owner.clone()))
        }
        CallableHolderAction::DropCaptureCellOwner { .. }
        | CallableHolderAction::DropCapturedValueCellOwner { .. }
        | CallableHolderAction::ReplaceCaptureCellValue { .. }
        | CallableHolderAction::RetainArrayElements { .. }
        | CallableHolderAction::SelectElement { .. } => None,
    }?;
    global_exit_holder_binding(&place)
}

fn exit_action_has_may_alias_guard(action: &CallableHolderAction) -> bool {
    let guards = match action {
        CallableHolderAction::DropSelected { guard, .. }
        | CallableHolderAction::RetainCheckedSelected { guard, .. }
        | CallableHolderAction::SkipUninitializedSelected { guard, .. } => guard,
        _ => return false,
    };
    guards
        .iter()
        .any(|guard| matches!(guard.relation, CallableArrayIndexRelation::MayAlias))
}

/// A MayAlias case needing different retain/drop actions on one global is a
/// source-qualified B0003 until separated by existing typed indices/loans.
fn exit_selected_ambiguity(actions: &[CallableHolderAction]) -> Option<SourceSpan> {
    let mut may_alias_span = None;
    for action in actions {
        if exit_action_has_may_alias_guard(action) {
            let span = match action {
                CallableHolderAction::DropSelected { span, .. }
                | CallableHolderAction::RetainCheckedSelected { span, .. }
                | CallableHolderAction::SkipUninitializedSelected { span, .. } => span.clone(),
                _ => continue,
            };
            may_alias_span = Some(span);
            break;
        }
    }
    let span = may_alias_span?;
    let mut envs: Vec<(ReferenceBindingId, BTreeSet<CallableEnvironmentId>)> = Vec::new();
    for action in actions {
        let (Some(binding), env) = (
            exit_selected_action_binding(action),
            match action {
                CallableHolderAction::Retain { environment, .. }
                | CallableHolderAction::Move { environment, .. }
                | CallableHolderAction::RetainSelected { environment, .. }
                | CallableHolderAction::MoveSelected { environment, .. }
                | CallableHolderAction::ReturnTransfer { environment, .. }
                | CallableHolderAction::Drop { environment, .. } => Some(environment.clone()),
                _ => None,
            },
        ) else {
            continue;
        };
        if !global_exit_binding_is_global(&binding) {
            continue;
        }
        if let Some(env) = env {
            if let Some(known) = envs.iter_mut().find(|known| known.0 == binding) {
                known.1.insert(env);
            } else {
                envs.push((binding, BTreeSet::from([env])));
            }
        }
    }
    for (_, environments) in &envs {
        if environments.len() > 1 {
            return Some(span);
        }
    }
    None
}
fn exit_origin_has_nonglobal_loan(cfg: &ScalarReferenceCfg, origin: &ReferenceOriginId) -> bool {
    match cfg.facts.get(origin.0) {
        Some(ReferenceOrigin::Fresh { loan, .. }) => {
            !global_exit_binding_is_global(&loan.origin_place.binding)
        }
        // A parameter-derived origin dies with the callee frame; only a
        // global referent outlives an installed global holder.
        Some(ReferenceOrigin::BorrowedFrom { .. }) => true,
        Some(ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. }) => false,
        None => true,
    }
}

fn exit_capture_has_nonglobal_loan(cfg: &ScalarReferenceCfg, input: &CallableCaptureInput) -> bool {
    input
        .origins
        .iter()
        .any(|origin| exit_origin_has_nonglobal_loan(cfg, origin))
        || input
            .backings
            .iter()
            .chain(
                input
                    .projected_backings
                    .iter()
                    .map(|projected| &projected.backing),
            )
            .any(|backing| exit_backing_has_nonglobal_loan(cfg, backing))
}

/// A wrong foreign or loan origin behind an installed global holder is a
/// source-qualified B0003: only a global referent outlives an installed
/// global holder, so a loan of a callee-local (or parameter-derived) place
/// must never publish through the call boundary.
fn exit_install_has_nonglobal_loan(
    cfg: &ScalarReferenceCfg,
    install: &CallableHolderRelation,
) -> bool {
    install.capture_paths.iter().flatten().any(|input| {
        input
            .origins
            .iter()
            .any(|origin| exit_origin_has_nonglobal_loan(cfg, origin))
            || input.environments.iter().any(|id| {
                cfg.environments.get(id.index).is_some_and(|fact| {
                    fact.loans
                        .iter()
                        .any(|loan| exit_origin_has_nonglobal_loan(cfg, &loan.origin))
                })
            })
            || exit_capture_has_nonglobal_loan(cfg, input)
    })
}

fn exit_backing_has_nonglobal_loan(cfg: &ScalarReferenceCfg, backing: &RuntimeBackingFact) -> bool {
    backing.callable_family.iter().any(|family| {
        family.elements.iter().any(|element| {
            element
                .loans
                .iter()
                .any(|loan| exit_origin_has_nonglobal_loan(cfg, &loan.origin))
        }) || family.partitions.iter().any(|partition| {
            partition
                .representatives
                .iter()
                .any(|representative| exit_cell_has_nonglobal_loan(cfg, &representative.cell))
                || exit_cell_has_nonglobal_loan(cfg, &partition.remainder)
        })
    })
}

fn exit_cell_has_nonglobal_loan(cfg: &ScalarReferenceCfg, cell: &CallableCellDomainFact) -> bool {
    cell.alternatives.iter().any(|case| match case {
        CallableCellCaseFact::Uninitialized => false,
        CallableCellCaseFact::Initialized {
            loans,
            capture_paths,
            ..
        } => {
            loans
                .iter()
                .any(|loan| exit_origin_has_nonglobal_loan(cfg, &loan.origin))
                || capture_paths.iter().flatten().any(|input| {
                    exit_capture_has_nonglobal_loan(cfg, input)
                        || input
                            .origins
                            .iter()
                            .any(|origin| exit_origin_has_nonglobal_loan(cfg, origin))
                })
        }
    })
}
fn exit_holder_backings_have_may_alias(
    holder_fact: &GlobalCallableCallPathFact,
) -> Option<SourceSpan> {
    for backing in &holder_fact.backings {
        for family in &backing.callable_family {
            if family
                .index_relations
                .iter()
                .any(|relation| matches!(relation.relation, CallableArrayIndexRelation::MayAlias))
            {
                return Some(backing.origin.clone());
            }
        }
    }
    None
}

fn exit_environments_have_may_alias(
    cfg: &ScalarReferenceCfg,
    binding: &ReferenceBindingId,
    environments: &BTreeSet<CallableEnvironmentId>,
) -> Option<SourceSpan> {
    for id in environments {
        let Some(fact) = cfg.environments.get(id.index) else {
            continue;
        };
        if fact.literal != id.literal {
            continue;
        }
        for backing in &fact.backings {
            // Only the changed holder's own storage counts: an unrelated
            // MayAlias family never forces this holder's retain/drop.
            if backing.owner != *binding {
                continue;
            }
            for family in &backing.callable_family {
                if family.index_relations.iter().any(|relation| {
                    matches!(relation.relation, CallableArrayIndexRelation::MayAlias)
                }) {
                    return Some(backing.origin.clone());
                }
            }
        }
    }
    None
}

fn build_global_call_exit_facts(
    cfg: &ScalarReferenceCfg,
    input: &GlobalCallableCallInputFact,
    writes: &[GlobalCallableOutputEffectFact],
    errors: &mut Vec<ReferenceAnalysisError>,
) -> Vec<GlobalCallableCallExitFact> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: input.call.clone(),
        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
    };
    if !validate_exit_input_provenance(cfg, input) {
        errors.push(invalid());
        return Vec::new();
    }
    let entry_relations: Vec<CallableHolderRelation> = input
        .holders
        .iter()
        .flat_map(|fact| fact.relations.iter().cloned())
        .collect();
    let mut facts = Vec::new();
    for exit in &cfg.holder_exits {
        if let Some(span) = exit_selected_ambiguity(&exit.actions) {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: span,
                reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
            });
            return Vec::new();
        }
        for relation in &exit.after {
            let Some(fact) = cfg.environments.get(relation.environment.index) else {
                errors.push(invalid());
                return Vec::new();
            };
            if fact.literal != relation.environment.literal {
                errors.push(invalid());
                return Vec::new();
            }
        }
        let mut post_holders = Vec::new();
        let mut kills = Vec::new();
        let mut installs = Vec::new();
        for holder_fact in &input.holders {
            let Some(binding) = global_exit_holder_binding(&holder_fact.holder) else {
                continue;
            };
            let relations = exit
                .after
                .iter()
                .filter(|relation| {
                    relation
                        .holders
                        .iter()
                        .any(|holder| holder_place_same(holder, &holder_fact.holder))
                })
                .cloned()
                .collect::<Vec<_>>();
            let targets_binding = writes.iter().any(|write| write.target.binding == binding);
            // Calls before initialization preserve the uninitialized alternative.
            let initialization = if !relations.is_empty() {
                GlobalCallableInitializationFact::Initialized
            } else if matches!(
                holder_fact.initialization,
                GlobalCallableInitializationFact::Uninitialized
            ) && !targets_binding
            {
                GlobalCallableInitializationFact::Uninitialized
            } else {
                GlobalCallableInitializationFact::Initialized
            };
            let entry_envs = holder_fact
                .relations
                .iter()
                .map(|relation| relation.environment.clone())
                .collect::<BTreeSet<_>>();
            let exit_envs = relations
                .iter()
                .map(|relation| relation.environment.clone())
                .collect::<BTreeSet<_>>();
            if matches!(
                initialization,
                GlobalCallableInitializationFact::Initialized
            ) && entry_envs != exit_envs
                && (targets_binding
                    || cfg
                        .nested_call_installs
                        .iter()
                        .any(|known| holder_place_same(known, &holder_fact.holder)))
            {
                // Kills and installs publish only what the callee wrote,
                // directly or through a nested applied call: entry/exit churn
                // without write provenance (scope drops, rebasing artifacts,
                // caller-owned locals such as top-level statement bindings)
                // must never cross the call boundary.
                // A MayAlias case needing different retain/drop actions is a
                // source-qualified B0003 until separated by typed indices/loans.
                // Ambiguity hides in the entry backings and in the callee
                // environment facts behind either generation.
                let ambiguous = exit_holder_backings_have_may_alias(holder_fact).or_else(|| {
                    exit_environments_have_may_alias(
                        cfg,
                        &binding,
                        &entry_envs.union(&exit_envs).cloned().collect(),
                    )
                });
                if let Some(origin) = ambiguous {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: origin,
                        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
                    });
                    return Vec::new();
                }
                kills.push(holder_fact.holder.clone());
                for relation in &relations {
                    if !entry_relations.contains(relation) && !installs.contains(relation) {
                        installs.push(relation.clone());
                    }
                }
            }
            post_holders.push(GlobalCallableCallPathFact {
                callee: holder_fact.callee.clone(),
                holder: holder_fact.holder.clone(),
                initialization,
                relations,
                backings: holder_fact.backings.clone(),
                projected_backings: holder_fact.projected_backings.clone(),
                origins: holder_fact.origins.clone(),
                released: holder_fact.released,
                moved: holder_fact.moved,
            });
        }
        // Cross-module mutations keep their actual validated states: a
        // callee-owned global written inside the callee but absent from the
        // correlated input alternative still publishes its post-call state.
        // Locals stay callee-internal.
        for write in writes {
            if !global_exit_binding_is_global(&write.target.binding) {
                continue;
            }
            let covered = input.holders.iter().any(|fact| {
                global_exit_holder_binding(&fact.holder)
                    .is_some_and(|binding| binding == write.target.binding)
            });
            if covered {
                continue;
            }
            let holder = if write.target.projections.is_empty() {
                CallableHolderPlace::Binding(write.target.binding.clone())
            } else {
                CallableHolderPlace::Projection(write.target.clone())
            };
            let relations = exit
                .after
                .iter()
                .filter(|relation| {
                    relation
                        .holders
                        .iter()
                        .any(|known| holder_place_same(known, &holder))
                })
                .cloned()
                .collect::<Vec<_>>();
            if relations.is_empty() {
                continue;
            }
            kills.push(holder.clone());
            for relation in &relations {
                if !entry_relations.contains(relation) && !installs.contains(relation) {
                    installs.push(relation.clone());
                }
            }
            post_holders.push(GlobalCallableCallPathFact {
                callee: input.target.clone(),
                holder,
                initialization: GlobalCallableInitializationFact::Initialized,
                relations,
                backings: Vec::new(),
                projected_backings: Vec::new(),
                origins: BTreeSet::new(),
                released: false,
                moved: false,
            });
        }
        // Installs stay global-only: locals released at callee scope exit are
        // callee-internal and never publish to the caller.
        installs.retain(|relation| {
            relation.holders.iter().any(|holder| {
                global_exit_holder_binding(holder)
                    .is_some_and(|binding| global_exit_binding_is_global(&binding))
            })
        });
        // A wrong foreign or loan origin behind an install is a
        // source-qualified B0003 with no published effect.
        if installs
            .iter()
            .any(|install| exit_install_has_nonglobal_loan(cfg, install))
        {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: input.call.clone(),
                reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
            });
            return Vec::new();
        }
        let fact = GlobalCallableCallExitFact {
            entry: input.occurrence.clone(),
            exit_point: exit.from,
            post_holders,
            kills,
            installs,
            actions: exit.actions.clone(),
            writes: writes.to_vec(),
        };
        if !facts.contains(&fact) {
            facts.push(fact);
        }
    }
    facts
}

fn rebase_exit_holder_action(
    action: &CallableHolderAction,
    environment_ids: &[CallableEnvironmentId],
    origin_ids: &[ReferenceOriginId],
) -> Option<CallableHolderAction> {
    match action {
        CallableHolderAction::SelectElement {
            target,
            index_point,
            selected,
        } => Some(CallableHolderAction::SelectElement {
            target: target.clone(),
            index_point: *index_point,
            selected: selected.clone(),
        }),
        CallableHolderAction::DropSelected {
            holder,
            previous,
            old,
            guard,
            span,
        } => Some(CallableHolderAction::DropSelected {
            holder: holder.clone(),
            previous: previous.clone(),
            old: rebase_capture_cell_case(old, environment_ids, origin_ids)?,
            guard: guard.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::SkipUninitializedSelected {
            holder,
            guard,
            span,
        } => Some(CallableHolderAction::SkipUninitializedSelected {
            holder: holder.clone(),
            guard: guard.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::RetainCheckedSelected {
            holder,
            old,
            guard,
            to,
            span,
        } => Some(CallableHolderAction::RetainCheckedSelected {
            holder: holder.clone(),
            old: rebase_capture_cell_case(old, environment_ids, origin_ids)?,
            guard: guard.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::RetainSelected {
            environment,
            from,
            to,
            span,
        } => Some(CallableHolderAction::RetainSelected {
            environment: environment_ids.get(environment.index)?.clone(),
            from: from.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::MoveSelected {
            environment,
            from,
            to,
            span,
        } => Some(CallableHolderAction::MoveSelected {
            environment: environment_ids.get(environment.index)?.clone(),
            from: from.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::Retain {
            environment,
            from,
            to,
            span,
        } => Some(CallableHolderAction::Retain {
            environment: environment_ids.get(environment.index)?.clone(),
            from: from.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::Move {
            environment,
            from,
            to,
            span,
        } => Some(CallableHolderAction::Move {
            environment: environment_ids.get(environment.index)?.clone(),
            from: from.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::Drop {
            environment,
            holder,
            span,
        } => Some(CallableHolderAction::Drop {
            environment: environment_ids.get(environment.index)?.clone(),
            holder: holder.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::DropArrayElements { backing, span } => {
            Some(CallableHolderAction::DropArrayElements {
                backing: rebase_mapped_backing(backing, environment_ids, origin_ids)?,
                span: span.clone(),
            })
        }
        CallableHolderAction::DropCapturedValueCellOwner {
            binding,
            cell,
            ty,
            span,
        } => Some(CallableHolderAction::DropCapturedValueCellOwner {
            binding: binding.clone(),
            cell: cell.clone(),
            ty: ty.clone(),
            span: span.clone(),
        }),
        CallableHolderAction::DropCaptureCellOwner {
            binding,
            cell,
            backing,
            span,
        } => Some(CallableHolderAction::DropCaptureCellOwner {
            binding: binding.clone(),
            cell: cell.clone(),
            backing: rebase_mapped_backing(backing, environment_ids, origin_ids)?,
            span: span.clone(),
        }),
        CallableHolderAction::ReplaceCaptureCellValue {
            binding,
            cell,
            previous,
            incoming,
            span,
        } => Some(CallableHolderAction::ReplaceCaptureCellValue {
            binding: binding.clone(),
            cell: cell.clone(),
            previous: rebase_mapped_backing(previous, environment_ids, origin_ids)?,
            incoming: rebase_mapped_backing(incoming, environment_ids, origin_ids)?,
            span: span.clone(),
        }),
        CallableHolderAction::RetainArrayElements {
            source,
            backing,
            span,
        } => Some(CallableHolderAction::RetainArrayElements {
            source: rebase_mapped_backing(source, environment_ids, origin_ids)?,
            backing: rebase_mapped_backing(backing, environment_ids, origin_ids)?,
            span: span.clone(),
        }),
        CallableHolderAction::ReturnTransfer {
            environment,
            from,
            to,
            span,
        } => Some(CallableHolderAction::ReturnTransfer {
            environment: environment_ids.get(environment.index)?.clone(),
            from: from.clone(),
            to: to.clone(),
            span: span.clone(),
        }),
    }
}

fn rebase_exit_relation(
    relation: &CallableHolderRelation,
    environment_ids: &[CallableEnvironmentId],
    origin_ids: &[ReferenceOriginId],
) -> Option<CallableHolderRelation> {
    let mut capture_paths = Vec::new();
    for path in &relation.capture_paths {
        capture_paths.push(rebase_mapped_capture_inputs(
            path,
            environment_ids,
            origin_ids,
        )?);
    }
    Some(CallableHolderRelation {
        environment: environment_ids.get(relation.environment.index)?.clone(),
        holders: relation.holders.clone(),
        capture_paths,
    })
}

fn exit_closure_env_ids(input: &CallableCaptureInput, ids: &mut BTreeSet<usize>) {
    for id in input
        .environments
        .iter()
        .chain(input.projected.iter().flat_map(|known| &known.environments))
    {
        ids.insert(id.index);
    }
    for backing in input
        .backings
        .iter()
        .chain(input.projected_backings.iter().map(|known| &known.backing))
    {
        exit_backing_env_ids(backing, ids);
    }
}

fn exit_backing_env_ids(backing: &RuntimeBackingFact, ids: &mut BTreeSet<usize>) {
    for family in &backing.callable_family {
        for element in &family.elements {
            ids.insert(element.environment.index);
        }
        for partition in &family.partitions {
            for representative in &partition.representatives {
                exit_cell_env_ids(&representative.cell, ids);
            }
            exit_cell_env_ids(&partition.remainder, ids);
        }
    }
}

fn exit_cell_env_ids(cell: &CallableCellDomainFact, ids: &mut BTreeSet<usize>) {
    for case in &cell.alternatives {
        if let CallableCellCaseFact::Initialized {
            environment_origin,
            capture_paths,
            ..
        } = case
        {
            ids.insert(environment_origin.index);
            for path in capture_paths {
                for input in path {
                    exit_closure_env_ids(input, ids);
                }
            }
        }
    }
}

fn exit_action_closure_env_ids(action: &CallableHolderAction, ids: &mut BTreeSet<usize>) {
    match action {
        CallableHolderAction::Retain { environment, .. }
        | CallableHolderAction::Move { environment, .. }
        | CallableHolderAction::RetainSelected { environment, .. }
        | CallableHolderAction::MoveSelected { environment, .. }
        | CallableHolderAction::ReturnTransfer { environment, .. }
        | CallableHolderAction::Drop { environment, .. } => {
            ids.insert(environment.index);
        }
        CallableHolderAction::RetainCheckedSelected { old, .. }
        | CallableHolderAction::DropSelected { old, .. } => {
            exit_cell_env_ids(
                &CallableCellDomainFact {
                    alternatives: vec![old.clone()],
                },
                ids,
            );
        }
        CallableHolderAction::DropArrayElements { backing, .. }
        | CallableHolderAction::DropCaptureCellOwner { backing, .. } => {
            exit_backing_env_ids(backing, ids);
        }
        CallableHolderAction::ReplaceCaptureCellValue {
            previous, incoming, ..
        } => {
            exit_backing_env_ids(previous, ids);
            exit_backing_env_ids(incoming, ids);
        }
        CallableHolderAction::RetainArrayElements {
            source, backing, ..
        } => {
            exit_backing_env_ids(source, ids);
            exit_backing_env_ids(backing, ids);
        }
        CallableHolderAction::SelectElement { .. }
        | CallableHolderAction::SkipUninitializedSelected { .. }
        | CallableHolderAction::DropCapturedValueCellOwner { .. } => {}
    }
}

/// Transitive callee-environment closure referenced by one exit fact: its
/// installs, connected actions and merged post-call backings. Only this
/// finite static set ever imports into the caller, so recursive equal-pair
/// reuse terminates instead of ping-ponging whole tables through the
/// positional input rebase.
fn exit_referenced_callee_envs(
    snapshot: &GlobalCallExitSnapshot,
    exit: &GlobalCallableCallExitFact,
) -> BTreeSet<usize> {
    let mut ids = BTreeSet::new();
    for relation in &exit.installs {
        ids.insert(relation.environment.index);
        for alternatives in &relation.capture_paths {
            for input in alternatives {
                exit_closure_env_ids(input, &mut ids);
            }
        }
    }
    for action in &exit.actions {
        exit_action_closure_env_ids(action, &mut ids);
    }
    for holder in &exit.post_holders {
        for backing in holder
            .backings
            .iter()
            .chain(holder.projected_backings.iter().map(|known| &known.backing))
        {
            exit_backing_env_ids(backing, &mut ids);
        }
    }
    ids.retain(|index| snapshot.environments.get(*index).is_some());
    // Transitive closure over the callee table so nested capture paths and
    // backing families never dangle after import.
    loop {
        let mut grown = false;
        for index in ids.clone() {
            let Some(fact) = snapshot.environments.get(index) else {
                continue;
            };
            let before = ids.len();
            for backing in &fact.backings {
                exit_backing_env_ids(backing, &mut ids);
            }
            for input in fact
                .inputs
                .iter()
                .chain(fact.capture_paths.iter().flatten())
            {
                exit_closure_env_ids(input, &mut ids);
            }
            ids.retain(|index| snapshot.environments.get(*index).is_some());
            if ids.len() != before {
                grown = true;
            }
        }
        if !grown {
            break;
        }
    }
    ids
}

fn global_exit_owner_source(owner: &ProducerCfgOwnerId) -> SourceIdentity {
    match owner {
        ProducerCfgOwnerId::Module(source) => source.clone(),
        ProducerCfgOwnerId::Function { source, .. } => source.clone(),
        ProducerCfgOwnerId::Literal { enclosing, .. } => global_exit_owner_source(enclosing),
    }
}

fn import_exit_environment_closure(
    cfg: &mut ScalarReferenceCfg,
    snapshot: &GlobalCallExitSnapshot,
    exit: &GlobalCallableCallExitFact,
) -> Option<Vec<CallableEnvironmentId>> {
    // Self-application reuses the shared table directly: a recursive
    // re-entry of an equal callee/canonical-state pair reuses its summary
    // without new abstract IDs. Tables only ever append, so the snapshot
    // prefix is exactly the live prefix; anything else is a missing edge.
    if snapshot.owner == cfg.producer_owner {
        return snapshot
            .environments
            .iter()
            .enumerate()
            .map(|(index, fact)| {
                if cfg
                    .environments
                    .get(index)
                    .is_some_and(|known| known.literal == fact.literal)
                {
                    Some(CallableEnvironmentId {
                        literal: fact.literal.clone(),
                        index,
                    })
                } else {
                    None
                }
            })
            .collect();
    }
    let closure = exit_referenced_callee_envs(snapshot, exit);
    let mut environment_ids: Vec<Option<CallableEnvironmentId>> =
        vec![None; snapshot.environments.len()];
    // Seed the closure map with already-imported destinations so recursive
    // re-entry of an equal pair reuses its summary without new abstract IDs.
    // When the callee itself imported an environment from this caller (mutual
    // recursion), resolve back to the caller's own entry instead of
    // re-importing the copy, which keeps the fixed point finite.
    for (index, fact) in snapshot.environments.iter().enumerate() {
        if let Some(origin) = snapshot.imports.iter().find(|known| {
            known.callee == cfg.producer_owner
                && known.destination.index == index
                && known.destination.literal == fact.literal
        }) {
            let Some(caller) = cfg.environments.get(origin.source_index) else {
                return None;
            };
            if caller.literal != origin.source_literal {
                return None;
            }
            environment_ids[index] = Some(CallableEnvironmentId {
                literal: caller.literal.clone(),
                index: origin.source_index,
            });
            continue;
        }
        if let Some(known) = cfg.global_call_exit_imports.iter().find(|known| {
            known.callee == snapshot.owner
                && known.source_index == index
                && known.source_literal == fact.literal
        }) {
            let destination = cfg.environments.get(known.destination.index)?;
            if destination.literal != known.destination.literal {
                return None;
            }
            environment_ids[index] = Some(known.destination.clone());
        }
    }
    // Allocate destinations for the remaining closure before rebasing any
    // nested content so E->C->E cycles terminate on the map.
    for index in closure {
        if environment_ids[index].is_some() {
            continue;
        }
        let fact = &snapshot.environments[index];
        let destination = CallableEnvironmentId {
            literal: fact.literal.clone(),
            index: cfg.environments.len(),
        };
        cfg.environments.push(CallableEnvironmentFact {
            literal: fact.literal.clone(),
            instance: fact.instance.clone(),
            origins: BTreeSet::new(),
            owners: BTreeSet::new(),
            loans: BTreeSet::new(),
            backings: Vec::new(),
            inputs: Vec::new(),
            capture_paths: Vec::new(),
        });
        cfg.global_call_exit_imports.push(GlobalCallExitImportFact {
            callee: snapshot.owner.clone(),
            source_index: index,
            source_literal: fact.literal.clone(),
            destination: destination.clone(),
        });
        environment_ids[index] = Some(destination);
    }
    // Dense source-table map: unmapped positions carry a poison entry that
    // rejects if ever read. The transitive closure above maps every position
    // reachable from the exit, so a poison read is a missing typed edge and
    // reports B0003 instead of inventing an owner.
    let poison = || CallableEnvironmentId {
        literal: SourceSpan::new(
            global_exit_owner_source(&snapshot.owner),
            ByteSpan::new(u32::MAX, u32::MAX),
        ),
        index: usize::MAX,
    };
    let mut dense = Vec::with_capacity(environment_ids.len());
    for slot in environment_ids {
        dense.push(slot.unwrap_or_else(&poison));
    }
    Some(dense)
}

fn fill_imported_exit_environments(
    cfg: &mut ScalarReferenceCfg,
    snapshot: &GlobalCallExitSnapshot,
    environment_ids: &[CallableEnvironmentId],
    origin_ids: &[ReferenceOriginId],
) -> Option<()> {
    for known in cfg.global_call_exit_imports.clone() {
        if known.callee != snapshot.owner {
            continue;
        }
        let source = snapshot.environments.get(known.source_index)?;
        if source.literal != known.source_literal {
            return None;
        }
        let rebased_origins = rebase_mapped_origins(&source.origins, origin_ids)?;
        let rebased_loans = rebase_mapped_loans(&source.loans, origin_ids)?;
        let mut rebased_backings = Vec::new();
        for backing in &source.backings {
            rebased_backings.push(rebase_mapped_backing(backing, environment_ids, origin_ids)?);
        }
        let rebased_inputs =
            rebase_mapped_capture_inputs(&source.inputs, environment_ids, origin_ids)?;
        let mut rebased_paths = Vec::new();
        for path in &source.capture_paths {
            rebased_paths.push(rebase_mapped_capture_inputs(
                path,
                environment_ids,
                origin_ids,
            )?);
        }
        let destination = cfg.environments.get_mut(known.destination.index)?;
        if destination.literal != known.destination.literal {
            return None;
        }
        destination.origins = rebased_origins;
        destination.owners = source.owners.clone();
        destination.loans = rebased_loans;
        destination.backings = rebased_backings;
        destination.inputs = rebased_inputs;
        destination.capture_paths = rebased_paths;
    }
    Some(())
}

fn resync_exit_holder_maps(state: &mut ReferenceEdgeState, holder: &CallableHolderPlace) {
    match holder {
        CallableHolderPlace::Binding(binding) => {
            let environments = state
                .holder_relations
                .iter()
                .filter(|relation| {
                    relation
                        .holders
                        .iter()
                        .any(|known| holder_place_same(known, holder))
                })
                .map(|relation| relation.environment.clone())
                .collect::<BTreeSet<_>>();
            if environments.is_empty() {
                state.callable_bindings.remove(binding);
            } else {
                state
                    .callable_bindings
                    .insert(binding.clone(), environments);
            }
        }
        CallableHolderPlace::Projection(place) => {
            let environments = state
                .holder_relations
                .iter()
                .filter(|relation| {
                    relation
                        .holders
                        .iter()
                        .any(|known| holder_place_same(known, holder))
                })
                .map(|relation| relation.environment.clone())
                .collect::<BTreeSet<_>>();
            if let Some(known) = state
                .callable_projected
                .iter_mut()
                .find(|known| reference_places_same(&known.place, place))
            {
                known.environments = environments;
            } else if !environments.is_empty() {
                state.callable_projected.push(CallableProjectedEnvironment {
                    place: place.clone(),
                    environments,
                });
            }
            state
                .callable_projected
                .retain(|known| !known.environments.is_empty());
        }
        _ => {}
    }
}

fn rebase_and_apply_call_exit(
    cfg: &mut ScalarReferenceCfg,
    snapshot: &GlobalCallExitSnapshot,
    exit: &GlobalCallableCallExitFact,
    state: &mut ReferenceEdgeState,
) -> Result<bool, ReferenceAnalysisError> {
    let invalid = || ReferenceAnalysisError::MissingContext {
        source_span: exit.entry.span.clone(),
        reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
    };
    let environment_ids =
        import_exit_environment_closure(cfg, snapshot, exit).ok_or_else(invalid)?;
    // Rebasing indexes by source-table position requires a dense map over the
    // callee table; unmapped positions reject as missing producers.
    let mut origin_ids: Vec<ReferenceOriginId> = Vec::new();
    for (index, origin) in snapshot.facts.iter().enumerate() {
        let position = cfg
            .facts
            .iter()
            .position(|known| known == origin)
            .unwrap_or_else(|| {
                let position = cfg.facts.len();
                cfg.facts.push(origin.clone());
                position
            });
        if origin_ids.len() != index {
            return Err(invalid());
        }
        origin_ids.push(ReferenceOriginId(position));
    }
    fill_imported_exit_environments(cfg, snapshot, &environment_ids, &origin_ids)
        .ok_or_else(invalid)?;
    // Self-application shares the table, so there is nothing to fill; every
    // other import fills through the closure map above.
    let mut rebased_installs = Vec::new();
    for install in &exit.installs {
        rebased_installs.push(
            rebase_exit_relation(install, &environment_ids, &origin_ids).ok_or_else(invalid)?,
        );
    }
    // Validate that every connected action rebases into the caller table;
    // a wrong foreign or loan origin rejects here as B0003. Rebased actions
    // are evidence only: runtime effects emit once where the write executes
    // inside the callee's protected mutation, never again in the caller.
    for action in &exit.actions {
        rebase_exit_holder_action(action, &environment_ids, &origin_ids).ok_or_else(invalid)?;
    }
    // Validate every rebased install against the caller table before the
    // imported fact becomes observable to transfer.
    for install in &rebased_installs {
        let Some(fact) = cfg.environments.get(install.environment.index) else {
            return Err(invalid());
        };
        if fact.literal != install.environment.literal {
            return Err(invalid());
        }
        for alternatives in &install.capture_paths {
            for input in alternatives {
                for id in input
                    .environments
                    .iter()
                    .chain(input.projected.iter().flat_map(|known| &known.environments))
                {
                    let Some(fact) = cfg.environments.get(id.index) else {
                        return Err(invalid());
                    };
                    if fact.literal != id.literal {
                        return Err(invalid());
                    }
                }
                for origin in &input.origins {
                    if cfg.facts.get(origin.0).is_none() {
                        return Err(invalid());
                    }
                }
            }
        }
    }
    let mut changed = false;
    // Kill and replace inside one path, then insert the whole resulting path.
    for kill in &exit.kills {
        // Uninitialized alternatives never kill: calls before initialization
        // preserve them alongside their diagnostic.
        let initialized = exit.post_holders.iter().any(|holder| {
            holder_place_same(&holder.holder, kill)
                && matches!(
                    holder.initialization,
                    GlobalCallableInitializationFact::Initialized
                )
        });
        if !initialized {
            continue;
        }
        let mut retained = Vec::new();
        for relation in state.holder_relations.drain(..) {
            if relation
                .holders
                .iter()
                .any(|holder| holder_place_same(holder, kill))
            {
                let survivors = relation
                    .holders
                    .iter()
                    .filter(|holder| !holder_place_same(holder, kill))
                    .cloned()
                    .collect::<Vec<_>>();
                if !survivors.is_empty() {
                    retained.push(CallableHolderRelation {
                        environment: relation.environment.clone(),
                        holders: survivors,
                        capture_paths: relation.capture_paths.clone(),
                    });
                }
                changed = true;
            } else {
                retained.push(relation);
            }
        }
        state.holder_relations = retained;
        state.holder_provenance.remove_holder(kill);
        resync_exit_holder_maps(state, kill);
    }
    normalize_holder_relations(&mut state.holder_relations);
    for install in &rebased_installs {
        if !state.holder_relations.contains(install) {
            state.holder_relations.push(install.clone());
            changed = true;
        }
        for holder in &install.holders {
            let Some(binding) = global_exit_holder_binding(holder) else {
                continue;
            };
            if !global_exit_binding_is_global(&binding) {
                continue;
            }
            // Record nested-application provenance so the next summary round
            // can republish transitively installed holders without mistaking
            // local entry/exit churn for a write.
            if !cfg
                .nested_call_installs
                .iter()
                .any(|known| holder_place_same(known, holder))
            {
                cfg.nested_call_installs.push(holder.clone());
            }
            state.holder_provenance.insert(
                install.environment.clone(),
                holder.clone(),
                HolderProducerRef::GlobalCallsiteInput {
                    call: exit.entry.span.clone(),
                    holder: holder.clone(),
                    environment: install.environment.clone(),
                },
            );
            resync_exit_holder_maps(state, holder);
        }
    }
    normalize_holder_relations(&mut state.holder_relations);
    // Merge the whole post-call global backing/origin state for installed holders.
    for holder in &exit.post_holders {
        if !matches!(
            holder.initialization,
            GlobalCallableInitializationFact::Initialized
        ) {
            continue;
        }
        let Some(binding) = global_exit_holder_binding(&holder.holder) else {
            continue;
        };
        if !global_exit_binding_is_global(&binding) {
            continue;
        }
        let installed = rebased_installs.iter().any(|install| {
            install
                .holders
                .iter()
                .any(|known| holder_place_same(known, &holder.holder))
        });
        if !installed {
            continue;
        }
        let rebased_origins = holder
            .origins
            .iter()
            .map(|origin| origin_ids.get(origin.0).copied().ok_or_else(invalid))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if !rebased_origins.is_empty() {
            let entry = state.bindings.entry(binding.clone()).or_default();
            let before = entry.len();
            entry.extend(rebased_origins);
            if entry.len() != before {
                changed = true;
            }
        }
        for backing in &holder.backings {
            let rebased = rebase_mapped_backing(backing, &environment_ids, &origin_ids)
                .ok_or_else(invalid)?;
            let entry = state
                .backing_bindings
                .entry(backing.owner.clone())
                .or_default();
            if !entry.contains(&rebased) {
                entry.push(rebased);
                changed = true;
            }
        }
        for projected in &holder.projected_backings {
            let rebased = rebase_mapped_projected_backing(projected, &environment_ids, &origin_ids)
                .ok_or_else(invalid)?;
            if !state.backing_projected.contains(&rebased) {
                state.backing_projected.push(rebased);
                changed = true;
            }
        }
    }
    Ok(changed)
}

fn apply_callee_exit_effects(
    cfg: &mut ScalarReferenceCfg,
    state: &mut ReferenceEdgeState,
    extra: &mut Vec<ReferenceEdgeState>,
    point: CfgPointId,
    target: &ResolvedCallTarget,
    output: usize,
    call_exits: &[GlobalCallExitSnapshot],
    errors: &mut Vec<ReferenceAnalysisError>,
) {
    for snapshot in call_exits {
        if snapshot.target != *target {
            continue;
        }
        let mut matched = Vec::new();
        for exit in &snapshot.exits {
            // Each exit fact applies only at its originating caller occurrence.
            if exit.entry.caller_cfg != cfg.producer_owner
                || exit.entry.point != point
                || exit.entry.output != output
                || exit.entry.callee != *target
            {
                continue;
            }
            if exit.entry.span != cfg.points[point.0].source_span {
                continue;
            }
            matched.push(exit);
        }
        if matched.is_empty() {
            continue;
        }
        // Exits with no kills and no installs carry no caller-observable
        // state change; skipping them keeps purely local forward trips out
        // of the global producer tables entirely.
        matched.retain(|exit| !exit.kills.is_empty() || !exit.installs.is_empty());
        if matched.is_empty() {
            continue;
        }
        // The first exit applies in place; further exits fan out as
        // whole-path successor alternatives.
        let base = state.clone();
        let mut first = true;
        for exit in matched {
            if first {
                first = false;
                match rebase_and_apply_call_exit(cfg, snapshot, exit, state) {
                    Ok(_) => {}
                    Err(error) => {
                        if !errors.contains(&error) {
                            errors.push(error);
                        }
                    }
                }
            } else {
                let mut alternative = base.clone();
                match rebase_and_apply_call_exit(cfg, snapshot, exit, &mut alternative) {
                    Ok(_) => {
                        if !extra.contains(&alternative) {
                            extra.push(alternative);
                        }
                    }
                    Err(error) => {
                        if !errors.contains(&error) {
                            errors.push(error);
                        }
                    }
                }
            }
        }
    }
}

fn resolve_runtime_backing_inputs(
    modules: &mut [ScalarModule],
    contracts: &[ConcreteReferenceOutputContract],
    literal_contracts: &[LiteralOutputContract],
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    let mut runtime_literal_contracts = literal_contracts.to_vec();
    let mut initial_transfer = true;
    loop {
        let updated_literal_contracts = match collect_project_literal_output_contracts(modules) {
            Ok(contracts) => contracts,
            Err(error) => return vec![error],
        };
        let mut paths = Vec::new();
        let mut global_calls = Vec::new();
        let mut output_contracts = Vec::new();
        for module in modules.iter() {
            collect_runtime_call_paths(&module.top_level_reference_cfg, &mut paths);
            collect_global_callable_inputs(&module.top_level_reference_cfg, &mut global_calls);
            for item in &module.items {
                if let ScalarItem::Function(function) = item {
                    collect_runtime_call_paths(&function.reference_cfg, &mut paths);
                    collect_global_callable_inputs(&function.reference_cfg, &mut global_calls);
                    collect_runtime_output_contracts(
                        &module.source,
                        function,
                        function.name_span,
                        RuntimeCallSelection::Declaration,
                        &mut output_contracts,
                    );
                }
            }
        }
        let mut changed =
            initial_transfer || runtime_literal_contracts != updated_literal_contracts;
        initial_transfer = false;
        runtime_literal_contracts = updated_literal_contracts;
        for module in modules.iter_mut() {
            changed |= install_runtime_output_contracts(
                &mut module.top_level_reference_cfg,
                &output_contracts,
            );
            for item in &mut module.items {
                if let ScalarItem::Function(function) = item {
                    match distribute_global_callable_inputs(&module.source, function, &global_calls)
                    {
                        Ok(updated) => changed |= updated,
                        Err(error) => return vec![error],
                    }
                    changed |= distribute_runtime_input_paths(function, &module.source, &paths);
                    changed |=
                        install_function_runtime_output_contracts(function, &output_contracts);
                }
            }
        }
        if !changed {
            let mut errors = Vec::new();
            for module in modules.iter() {
                unresolved_runtime_outputs(
                    &module.top_level_reference_cfg,
                    &runtime_literal_contracts,
                    &mut errors,
                );
                for item in &module.items {
                    if let ScalarItem::Function(function) = item {
                        unresolved_function_runtime_outputs(
                            function,
                            &runtime_literal_contracts,
                            &mut errors,
                        );
                    }
                }
            }
            return errors;
        }
        // Cut 3: inner transfer/exit fixed point. Transfers apply the
        // previous round's same-occurrence exit summaries at originating
        // caller occurrences; refresh rebuilds summaries from the fresh
        // holder exits. Equal facts stop the fixed point over the finite
        // canonical abstract state universe.
        loop {
            // Same-occurrence exit summaries from the previous round apply
            // at originating caller occurrences inside this round's dataflow.
            let call_exit_snapshots = collect_global_call_exit_snapshots(modules);
            let mut errors = Vec::new();
            for module in modules.iter_mut() {
                let forest = match source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete) { Ok(forest) => forest, Err(error) => return vec![error] };
                let module_input = match source_boundary::preflight_input(&module.top_level_reference_cfg.producer_owner, &forest, BTreeMap::new()) { Ok(input) => input, Err(error) => return vec![error] };
                if let Err(error) = source_boundary::validate_capture_preflight(&module.top_level_reference_cfg, &module_input) { return vec![error]; }
                module.top_level_reference_cfg.points =
                    module.top_level_reference_cfg.initial_points.clone();
                errors.extend(transfer_reference_facts(
                    &mut module.top_level_reference_cfg,
                    &BTreeMap::new(),
                    contracts,
                    &runtime_literal_contracts,
                    &call_exit_snapshots,
                    &module_input,
                    #[cfg(test)]
                    ReferenceTransferTestRole::TopLevel,
                    #[cfg(test)]
                    trace,
                ));
                errors.extend(refresh_literal_cfgs(
                    &mut module.top_level_reference_cfg,
                    contracts,
                    &runtime_literal_contracts,
                    &module_input,
                    #[cfg(test)]
                    trace,
                ));
                for item in &mut module.items {
                    if let ScalarItem::Function(function) = item {
                        let capture_input = match source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()) { Ok(input) => input, Err(error) => return vec![error] };
                        errors.extend(transfer_function_output_contracts(
                            function,
                            contracts,
                            &runtime_literal_contracts,
                            &call_exit_snapshots,
                            &capture_input,
                            #[cfg(test)]
                            trace,
                        ));
                        errors.extend(refresh_function_literal_cfgs(
                            function,
                            contracts,
                            &runtime_literal_contracts,
                            &capture_input,
                            #[cfg(test)]
                            trace,
                        ));
                    }
                }
            }
            // Rebuild same-occurrence exit summaries from the fresh holder
            // exits; equal facts stop the fixed point.
            let mut exit_errors = Vec::new();
            let mut exit_changed = false;
            for module in modules.iter_mut() {
                for item in &mut module.items {
                    if let ScalarItem::Function(function) = item {
                        exit_changed |=
                            refresh_function_call_exit_facts(function, &mut exit_errors);
                    }
                }
            }
            errors.extend(exit_errors);
            if !errors.is_empty() {
                return errors;
            }
            if !exit_changed {
                break;
            }
            // No caller-observable content anywhere: further rounds can only
            // churn local transfer state that the base single-pass flow
            // already resolved. Break to preserve the base recorded outcome
            // for purely local call graphs.
            let mut has_content = false;
            for module in modules.iter() {
                for item in &module.items {
                    if let ScalarItem::Function(function) = item {
                        if function
                            .reference_cfg
                            .global_call_exit_facts
                            .iter()
                            .any(|fact| !fact.kills.is_empty() || !fact.installs.is_empty())
                        {
                            has_content = true;
                            break;
                        }
                        for arm in &function.overload_arms {
                            if arm
                                .reference_cfg
                                .global_call_exit_facts
                                .iter()
                                .any(|fact| !fact.kills.is_empty() || !fact.installs.is_empty())
                            {
                                has_content = true;
                                break;
                            }
                        }
                    }
                    if has_content {
                        break;
                    }
                }
                if has_content {
                    break;
                }
            }
            if !has_content {
                break;
            }
        }
    }
}

fn record_runtime_call_signatures(
    source: &SourceIdentity,
    function: &ScalarFunction,
    declaration_span: ByteSpan,
    signatures: &mut Vec<RuntimeCallSignature>,
) {
    if let ScalarType::Callable {
        parameters,
        outputs,
    } = &function.signature
    {
        signatures.push(RuntimeCallSignature {
            source: source.clone(),
            declaration_span,
            selection: RuntimeCallSelection::Declaration,
            parameters: parameters.clone(),
            outputs: outputs
                .outputs
                .iter()
                .map(|output| output.ty.clone())
                .collect(),
            generic_parameters: function
                .generic_parameters
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect(),
            bindings: function
                .parameter_spans
                .iter()
                .map(|span| ReferenceBindingId {
                    source: source.clone(),
                    function_span: function.span,
                    declaration_span: *span,
                    block_span: function.body.span,
                    kind: ReferenceBindingKind::Declared,
                })
                .collect(),
        });
    }
    for (index, arm) in function.overload_arms.iter().enumerate() {
        if let ScalarType::Callable {
            parameters,
            outputs,
        } = &arm.signature
        {
            signatures.push(RuntimeCallSignature {
                source: source.clone(),
                declaration_span,
                selection: RuntimeCallSelection::Overload(index),
                parameters: parameters.clone(),
                outputs: outputs
                    .outputs
                    .iter()
                    .map(|output| output.ty.clone())
                    .collect(),
                generic_parameters: arm
                    .generic_parameters
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect(),
                bindings: arm
                    .parameter_spans
                    .iter()
                    .map(|span| ReferenceBindingId {
                        source: source.clone(),
                        function_span: arm.span,
                        declaration_span: *span,
                        block_span: arm.body.span,
                        kind: ReferenceBindingKind::Declared,
                    })
                    .collect(),
            });
        }
    }
}

fn install_runtime_call_signatures(
    function: &mut ScalarFunction,
    signatures: &[RuntimeCallSignature],
) {
    function.reference_cfg.call_signatures = signatures.to_vec();
    for literal in &mut function.reference_cfg.literal_cfgs {
        literal.reference_cfg.call_signatures = signatures.to_vec();
    }
    for arm in &mut function.overload_arms {
        install_runtime_call_signatures(arm, signatures);
    }
}

pub(super) fn record_specialized_copy_policies(
    items: &mut [ScalarItem],
    modules: &[ScalarModule],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for item in items {
        if let ScalarItem::Function(function) = item {
            errors.extend(record_function_specialized_copy_policies(
                function,
                modules,
                structs,
                enums,
                #[cfg(test)]
                trace,
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
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for arm in &mut function.overload_arms {
        errors.extend(record_function_specialized_copy_policies(
            arm,
            modules,
            structs,
            enums,
            #[cfg(test)]
            trace,
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
        if !deferred.is_empty() || !template.reference_cfg.source_capture_inventory.required.is_empty() || template.reference_cfg.evaluated_values.disposition_inputs.iter()
            .any(|input| matches!(input.consumer.kind, ValueConsumerKind::Binding { .. })) {
            let mut concrete_cfg = template.reference_cfg.clone();
            let defining_module = modules.iter().find(|module| module.source == *source).expect("resolved defining module");
            let capture_input = match source_boundary::module_preflight_forest(&defining_module.source, &defining_module.items, &defining_module.top_level_reference_cfg, &defining_module.structs, &defining_module.enums, SourceCaptureConstructionPhase::Complete)
                .and_then(|forest| source_boundary::preflight_input(&template.reference_cfg.producer_owner, &forest, substitutions.clone())) { Ok(input) => input, Err(error) => { errors.push(error); continue; } };
            if let Err(error) = source_boundary::install_capture_policy_instances(&mut concrete_cfg, &capture_input)
                .and_then(|()| source_boundary::validate_capture_preflight(&concrete_cfg, &capture_input)) { errors.push(error); continue; }
            concrete_cfg.points = concrete_cfg.initial_points.clone();
            let mut policy_errors = Vec::new();
            for input in &mut concrete_cfg.evaluated_values.disposition_inputs {
                if !matches!(input.consumer.kind, ValueConsumerKind::Binding { .. }) { continue; }
                let facts = concrete_cfg.evaluated_values.values.iter().filter(|fact| fact.id == input.source).collect::<Vec<_>>();
                let [fact] = facts.as_slice() else { policy_errors.push(error()); continue; };
                let shapes = concrete_cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
                let [shape] = shapes.as_slice() else { policy_errors.push(error()); continue; };
                let concrete_ty = substitute_type(&shape.ty, &substitutions);
                if !scalar_type_equal(&shape.ty, &concrete_ty) {
                    policy_errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: input.source.resolved.occurrence.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    });
                    continue;
                }
                match copy_policy(&concrete_ty, structs, enums, &generic_parameters) {
                    Ok(policy) => input.policy = policy,
                    Err(_) => policy_errors.push(error()),
                }
            }
            if !policy_errors.is_empty() { errors.extend(policy_errors); continue; }
            let transferred = transfer_reference_facts(
                &mut concrete_cfg,
                &deferred,
                &[],
                &[],
                &[],
                &capture_input,
                #[cfg(test)]
                ReferenceTransferTestRole::SpecializedCopy {
                    types: template
                        .generic_parameters
                        .iter()
                        .map(|parameter| substitutions[&parameter.name].clone())
                        .collect(),
                },
                #[cfg(test)]
                trace,
            );
            errors.extend(transferred.into_iter().map(|error| match error {
                conflict @ ReferenceAnalysisError::ConflictingPlace { .. } => conflict,
                error => ReferenceAnalysisError::MissingContext {
                    source_span: point.source_span.clone(),
                    reason: error.reason(),
                },
            }));
        }
    }
    errors
}

pub(super) fn record_function_reference_origins(
    source: &SourceIdentity,
    function: &mut ScalarFunction,
    module_names: &HashSet<String>,
    global_bindings: &BTreeMap<String, (ReferenceBindingId, ScalarType)>,
    call_sites: &[ResolvedCallSite],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
    literal_signatures: &[closure::ResolvedLiteralSignature],
    value_types: &assignment::SourceValueTypeFacts,
) -> Vec<ReferenceAnalysisError> {
    record_function_reference_origins_worker(
        source,
        function,
        module_names,
        global_bindings,
        &[],
        &[],
        call_sites,
        structs,
        enums,
        assignment_outputs,
        literal_signatures,
        value_types,
        #[cfg(test)]
        &mut ReferenceValidationTestTrace::new(),
    )
}

fn record_function_reference_origins_worker(
    source: &SourceIdentity,
    function: &mut ScalarFunction,
    module_names: &HashSet<String>,
    global_bindings: &BTreeMap<String, (ReferenceBindingId, ScalarType)>,
    name_globals: &[SourceNameGlobalDeclaration],
    name_originals: &[SourceNameResolutionContextRoot],
    call_sites: &[ResolvedCallSite],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
    literal_signatures: &[closure::ResolvedLiteralSignature],
    value_types: &assignment::SourceValueTypeFacts,
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for arm in &mut function.overload_arms {
        errors.extend(record_function_reference_origins_worker(
            source,
            arm,
            module_names,
            global_bindings,
            name_globals,
            name_originals,
            call_sites,
            structs,
            enums,
            assignment_outputs,
            literal_signatures,
            value_types,
            #[cfg(test)]
            trace,
        ));
    }
    let mut builder = ReferenceFlowBuilder {
        rebind_association_count: 0,
        rebind_associations: BTreeMap::new(),
        source: source.clone(),
        function_span: function.span,
        assignment_context_span: function.span,
        names: BTreeMap::new(),
        global_bindings: global_bindings.clone(),
        module_names: module_names.clone(),
        call_sites: call_sites.to_vec(),
        origins: HashMap::new(),
        checked: HashSet::new(),
        binding_types: global_bindings
            .values()
            .map(|(binding, ty)| (binding.clone(), ty.clone()))
            .collect(),
        structs: structs.to_vec(),
        enums: enums.to_vec(),
        generic_scope: function.reference_cfg.producer_owner.clone(),
        generic_declarations: function.generic_parameters.clone(),
        generic_parameters: function
            .generic_parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect(),
        assignment_outputs: assignment_outputs.clone(),
        value_types: value_types.clone(),
        literal_signatures: literal_signatures.to_vec(),
        dereference_points: HashMap::new(),
        index_points: HashMap::new(),
        array_elements: Box::new(HashMap::new()),
        cfg: ScalarReferenceCfg {
            source_capture_inventory: SourceCaptureInventory::building(),
            name_resolution_context: NameResolutionContext::Building,
            source_null_constructors: Vec::new(),
            module_resolver_input: source_boundary::SourceModuleResolverInputState::NotApplicable,
            evaluated_values: EvaluatedValueTables::new(),
            producer_owner: function.reference_cfg.producer_owner.clone(),
            facts: Vec::new(),
            borrow_evaluations: Vec::new(),
            rebind_associations: Vec::new(),
            allocation_address_evaluations: Vec::new(),
            rebind_evaluations: Vec::new(),
            checked_installs: Vec::new(),
            checked_install_evaluations: Vec::new(),
            checked_bindings: BTreeMap::new(),
            environments: Vec::new(),
            environment_imports: Vec::new(),
            inputs: Vec::new(),
            capture_input_paths: Vec::new(),
            capture_input_path_facts: Vec::new(),
            producer_return_cases: Vec::new(),
            global_callable_input_paths: Vec::new(),
            global_call_exit_facts: Vec::new(),
            global_call_exit_imports: Vec::new(),
            nested_call_installs: Vec::new(),
            backing_parameters: Vec::new(),
            callable_array_parameters: Vec::new(),
            backing_length_sites: Vec::new(),
            backing_input_paths: Vec::new(),
            projected_backing_input_paths: Vec::new(),
            call_signatures: Vec::new(),
            backing_output_contracts: Vec::new(),
            literal_cfgs: Vec::new(),
            points: Vec::new(),
            while_points: Vec::new(),
            block_scope_exits: Vec::new(),
            conditional_points: Vec::new(),
            call_points: Vec::new(),
            capture_points: Vec::new(),
            read_points: Vec::new(),
            binding_points: Vec::new(),
            assignment_points: Vec::new(),
            top_level: Box::default(),
            holder_edges: Vec::new(),
            holder_exits: Vec::new(),
            initial_points: Vec::new(),
            allocations: BTreeSet::new(),
            reference_parameters: Vec::new(),
            specialized_policies: Vec::new(),
        },
        next_scope: 0,
        cursor: None,
    };
    builder.cfg.name_resolution_context = match name_resolution::named_context(source, function, module_names, global_bindings, name_globals, name_originals, &builder.value_types.expressions) {
        Ok(context) => context,
        Err(error) => { errors.push(error); return errors; },
    };
    builder.point(
        CfgPointKind::Entry,
        function.span,
        ReferenceScopeId(0),
        BTreeSet::new(),
    );
    builder.checked_global_storage();
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
    builder.cfg.reference_parameters = function
        .parameters
        .iter()
        .map(|name| builder.names[name].clone())
        .collect();
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
            if matches!(ty, ScalarType::RuntimeArray { .. }) {
                builder
                    .cfg
                    .backing_parameters
                    .push(builder.names[name].clone());
                if let ScalarType::RuntimeArray { element, .. } = ty {
                    if matches!(element.as_ref(), ScalarType::Callable { .. }) {
                        builder
                            .cfg
                            .callable_array_parameters
                            .push(CallableArrayParameterFact {
                                binding: builder.names[name].clone(),
                                element: *element.clone(),
                            });
                    }
                }
            }
            if let ScalarType::CheckedReference { mutability, .. } = ty {
                let binding = builder.names[name].clone();
                builder.checked_storage(
                    &binding,
                    ScalarPlace::Name {
                        name: name.clone(),
                        span: *span,
                    },
                );
                let mut referent = binding.clone();
                referent.kind = ReferenceBindingKind::BorrowedReferent { parameter: index };
                let loan = ReferenceLoanId {
                    mode: *mutability,
                    origin_place: ReferencePlaceId {
                        source: source.clone(),
                        function_span: function.span,
                        declaration_span: *span,
                        block_span: function.body.span,
                        binding: referent,
                        projections: Vec::new(),
                        place: ScalarPlace::Name {
                            name: name.clone(),
                            span: *span,
                        },
                    },
                    creation_source: source.clone(),
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
                let point = builder.point(
                    CfgPointKind::Bind {
                        binding: binding.clone(),
                    },
                    *span,
                    ReferenceScopeId(0),
                    BTreeSet::from([id]),
                );
                builder.cfg.checked_installs.push(ReferenceCheckedInstall {
                    point,
                    input: ReferenceCheckedInstallInput::Parameter {
                        binding,
                        origin: id,
                    },
                });
                builder.cfg.evaluated_values.source_origin_derivations.push(source_boundary::checked_parameter_constructor::derivation(
                    &builder.cfg.producer_owner, point, id, index, builder.cfg.reference_parameters[index].clone(),
                ));
            }
        }
    }
    if let Err(error) = builder.introduce_nonchecked_parameters(&function.signature) {
        errors.push(error);
        return errors;
    }
    let ScalarType::Callable { outputs, .. } = &function.signature else {
        return vec![builder.missing_context(
            function.span,
            ReferenceAnalysisErrorReason::MissingResolvedBinding,
        )];
    };
    match builder.block_outputs(&function.body, &outputs.outputs) {
        Err(error) => errors.push(error),
        Ok(result) => {
            if let ScalarType::Callable { outputs, .. } = &function.signature {
                if matches!(
                    outputs.outputs.first(),
                    Some(ScalarOutput {
                        ty: ScalarType::CheckedReference { .. },
                        ..
                    })
                ) && result.value.is_none()
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
        if let Err(error) =
            builder.register_boundary_signature(&function.signature, &[], global_bindings)
        {
            errors.push(error);
            return errors;
        }
        if let Err(error) = owner_lifetime::register(&mut builder.cfg) {
            errors.push(error);
            return errors;
        }
        builder.publish_rebind_associations();
        builder.cfg.initial_points = builder.cfg.points.clone();
        if let Err(error) = name_resolution::validate_named_source(&builder.cfg, source, function, module_names, global_bindings, name_globals, name_originals, &builder.value_types.expressions) { errors.push(error); return errors; }
        if let Err(error) = name_resolution::validate(&builder.cfg, structs) { errors.push(error); return errors; }
        if let Err(error) = selected_source::register_literal_members(&mut builder.cfg, structs, &builder.generic_scope, &builder.generic_declarations) {
            errors.push(error);
            return errors;
        }
        let capture_input = match source_boundary::local_preflight_input(&builder.cfg, structs, enums, &builder.generic_parameters, &builder.generic_declarations) { Ok(input) => input, Err(error) => { errors.push(error); return errors; } };
        let initial_transfer = transfer_reference_facts_observed(
            &mut builder.cfg,
            &BTreeMap::new(),
            &[],
            &[],
            &[],
            &capture_input,
            #[cfg(test)]
            ReferenceTransferTestRole::Function,
            #[cfg(test)]
            ReferenceTransferTestStage::InitialConstruction,
            #[cfg(test)]
            trace,
        );
        errors.extend(record_literal_reference_cfgs(
            &mut builder.cfg,
            &capture_input,
            &initial_transfer,
            source,
            function.span,
            &builder.generic_scope,
            &builder.generic_declarations,
            &builder.generic_parameters,
            module_names,
            global_bindings,
            call_sites,
            structs,
            enums,
            assignment_outputs,
            literal_signatures,
            value_types,
            #[cfg(test)]
            trace,
        ));
        if errors.is_empty() {
            let completion = source_boundary::local_preflight_input(&builder.cfg, structs, enums, &builder.generic_parameters, &builder.generic_declarations)
                .and_then(|input| source_boundary::complete_capture_children(&mut builder.cfg, &input)
                    .and_then(|()| source_boundary::seal_capture_tree(&mut builder.cfg, &input.policy_context)));
            if let Err(error) = completion { errors.push(error); }
        }
    }
    function.reference_cfg = builder.cfg;
    errors
}

fn record_literal_reference_cfgs(
    parent: &mut ScalarReferenceCfg,
    ancestor_input: &source_boundary::SourceCapturePreflightInput,
    initial_transfer: &name_resolution::initial_transfer::InitialTransferReceipt,
    source: &SourceIdentity,
    assignment_context_span: ByteSpan,
    generic_scope: &ProducerCfgOwnerId,
    generic_declarations: &[ScalarGenericParameter],
    generic_parameters: &BTreeSet<String>,
    module_names: &HashSet<String>,
    global_bindings: &BTreeMap<String, (ReferenceBindingId, ScalarType)>,
    call_sites: &[ResolvedCallSite],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
    literal_signatures: &[closure::ResolvedLiteralSignature],
    value_types: &assignment::SourceValueTypeFacts,
    #[cfg(test)] trace: &mut ReferenceValidationTestTrace,
) -> Vec<ReferenceAnalysisError> {
    if let Err(error) = name_resolution::initial_transfer::validate_handoff(parent, ancestor_input, initial_transfer) { return vec![error]; }
    if !parent.literal_cfgs.is_empty() || !parent.evaluated_values.source_literal_constructor_seeds.is_empty() {
        return vec![ReferenceAnalysisError::MissingContext { source_span: parent.initial_points[0].source_span.clone(), reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence }];
    }
    let mut errors = Vec::new();
    let mut literal_cfgs = Vec::new();
    for point in &parent.initial_points {
        let CfgPointKind::Capture {
            literal,
            environment,
            signature,
            captures,
            parameters,
            parameter_spans,
            body,
            ..
        } = &point.kind
        else {
            continue;
        };
        let declaration = match source_epoch::SourceCaptureDeclaration::new(
            &parent.producer_owner,
            &point.source_span,
            point.id,
            *literal,
            &environment.literal,
            signature,
            captures,
            parameters,
            parameter_spans,
            body,
        ) {
            Ok(declaration) => declaration,
            Err(_) => {
                errors.push(ReferenceAnalysisError::MissingContext {
                    source_span: point.source_span.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                });
                continue;
            }
        };
        let outputs = declaration.outputs;
        let constructor_seed = match name_resolution::literal_seed::freeze(parent, ancestor_input, initial_transfer, point, &declaration.literal_owner) {
            Ok(seed) => seed,
            Err(error) => { errors.push(error); continue; },
        };
        let captured = &parent.environments[environment.index];
        let mut builder = ReferenceFlowBuilder {
            rebind_association_count: 0,
            rebind_associations: BTreeMap::new(),
            source: declaration.literal_span.source.clone(),
            function_span: declaration.literal_span.range,
            assignment_context_span,
            names: BTreeMap::new(),
            global_bindings: global_bindings.clone(),
            module_names: module_names.clone(),
            call_sites: call_sites.to_vec(),
            origins: constructor_seed.inputs
                .iter()
                .filter(|input| !input.origins.is_empty())
                .map(|input| (input.binding.clone(), input.origins.clone()))
                .collect(),
            checked: HashSet::new(),
            binding_types: global_bindings
                .values()
                .map(|(binding, ty)| (binding.clone(), ty.clone()))
                .collect(),
            structs: structs.to_vec(),
            enums: enums.to_vec(),
            generic_scope: generic_scope.clone(),
            generic_declarations: generic_declarations.to_vec(),
            generic_parameters: generic_parameters.clone(),
            assignment_outputs: assignment_outputs.clone(),
            value_types: value_types.clone(),
            literal_signatures: literal_signatures.to_vec(),
            dereference_points: HashMap::new(),
            index_points: HashMap::new(),
            array_elements: Box::new(HashMap::new()),
            cfg: ScalarReferenceCfg {
                source_capture_inventory: SourceCaptureInventory::building(),
                name_resolution_context: NameResolutionContext::Building,
                source_null_constructors: Vec::new(),
                module_resolver_input: source_boundary::SourceModuleResolverInputState::NotApplicable,
                evaluated_values: EvaluatedValueTables::new(),
                producer_owner: declaration.literal_owner.clone(),
                facts: constructor_seed.inherited_facts.clone(),
                borrow_evaluations: Vec::new(),
                rebind_associations: Vec::new(),
                allocation_address_evaluations: Vec::new(),
                rebind_evaluations: Vec::new(),
                checked_installs: Vec::new(),
                checked_install_evaluations: Vec::new(),
                checked_bindings: BTreeMap::new(),
                environments: parent.environments.clone(),
                environment_imports: parent.environment_imports.clone(),
                inputs: constructor_seed.inputs.clone(),
                capture_input_paths: captured.capture_paths.clone(),
                capture_input_path_facts: parent
                    .capture_input_path_facts
                    .iter()
                    .filter(|fact| fact.capture == point.id)
                    .cloned()
                    .collect(),
                producer_return_cases: Vec::new(),
                global_callable_input_paths: Vec::new(),
                global_call_exit_facts: Vec::new(),
                global_call_exit_imports: Vec::new(),
                nested_call_installs: Vec::new(),
                backing_parameters: Vec::new(),
                callable_array_parameters: Vec::new(),
                backing_length_sites: Vec::new(),
                backing_input_paths: Vec::new(),
                projected_backing_input_paths: Vec::new(),
                call_signatures: Vec::new(),
                backing_output_contracts: Vec::new(),
                literal_cfgs: Vec::new(),
                points: Vec::new(),
                while_points: Vec::new(),
                block_scope_exits: Vec::new(),
                conditional_points: Vec::new(),
                call_points: Vec::new(),
                capture_points: Vec::new(),
                read_points: Vec::new(),
                binding_points: Vec::new(),
                assignment_points: Vec::new(),
                top_level: Box::default(),
                holder_edges: Vec::new(),
                holder_exits: Vec::new(),
                initial_points: Vec::new(),
                allocations: parent.allocations.clone(),
                reference_parameters: Vec::new(),
                specialized_policies: Vec::new(),
            },
            next_scope: 0,
            cursor: None,
        };
        builder.cfg.name_resolution_context = match name_resolution::literal_context(parent, &declaration, global_bindings, &builder.value_types.expressions) {
            Ok(context) => context,
            Err(error) => { errors.push(error); continue; },
        };
        for capture in declaration.captures {
            builder
                .names
                .insert(capture.name.clone(), capture.binding.clone());
            builder
                .binding_types
                .insert(capture.binding.clone(), capture.ty.clone());
            if matches!(capture.ty, ScalarType::CheckedReference { .. }) {
                builder.checked.insert(capture.binding.clone());
                let storage = parent
                    .checked_bindings
                    .get(&capture.binding)
                    .expect("resolved checked capture storage")
                    .clone();
                builder
                    .cfg
                    .checked_bindings
                    .insert(capture.binding.clone(), storage);
            }
        }
        builder.point(
            CfgPointKind::Entry,
            declaration.literal_span.range,
            ReferenceScopeId(0),
            BTreeSet::new(),
        );
        builder.checked_global_storage();
        for parameter in &declaration.parameters {
            let index = parameter.ordinal;
            let name = parameter.name;
            let ty = parameter.ty;
            let binding = parameter.binding.clone();
            let span = binding.declaration_span;
            builder.names.insert(name.clone(), binding.clone());
            builder.binding_types.insert(binding.clone(), ty.clone());
            builder.cfg.reference_parameters.push(binding.clone());
            if matches!(ty, ScalarType::RuntimeArray { .. }) {
                builder.cfg.backing_parameters.push(binding.clone());
                if let ScalarType::RuntimeArray { element, .. } = ty {
                    if matches!(element.as_ref(), ScalarType::Callable { .. }) {
                        builder
                            .cfg
                            .callable_array_parameters
                            .push(CallableArrayParameterFact {
                                binding: binding.clone(),
                                element: *element.clone(),
                            });
                    }
                }
            }
            if let ScalarType::CheckedReference { mutability, .. } = ty {
                builder.checked_storage(
                    &binding,
                    ScalarPlace::Name {
                        name: name.clone(),
                        span,
                    },
                );
                let mut referent = binding.clone();
                referent.kind = ReferenceBindingKind::BorrowedReferent { parameter: index };
                let loan = ReferenceLoanId {
                    mode: *mutability,
                    origin_place: ReferencePlaceId {
                        source: binding.source.clone(),
                        function_span: binding.function_span,
                        declaration_span: span,
                        block_span: binding.block_span,
                        binding: referent,
                        projections: Vec::new(),
                        place: ScalarPlace::Name {
                            name: name.clone(),
                            span,
                        },
                    },
                    creation_source: binding.source.clone(),
                    creation_span: span,
                    parent: None,
                };
                let origin = builder.intern(ReferenceOrigin::BorrowedFrom {
                    parameter: index,
                    loan,
                });
                builder.checked.insert(binding.clone());
                builder
                    .origins
                    .insert(binding.clone(), BTreeSet::from([origin]));
                let point = builder.point(
                    CfgPointKind::Bind {
                        binding: binding.clone(),
                    },
                    span,
                    ReferenceScopeId(0),
                    BTreeSet::from([origin]),
                );
                builder.cfg.checked_installs.push(ReferenceCheckedInstall {
                    point,
                    input: ReferenceCheckedInstallInput::Parameter { binding, origin },
                });
                builder.cfg.evaluated_values.source_origin_derivations.push(source_boundary::checked_parameter_constructor::derivation(
                    &builder.cfg.producer_owner, point, origin, index, builder.cfg.reference_parameters[index].clone(),
                ));
            }
        }
        if let Err(error) = builder.introduce_nonchecked_parameters(declaration.signature) {
            errors.push(error);
            continue;
        }
        let mut typed_body = declaration.body.clone();
        for (value, expected) in typed_body
            .final_output_values
            .iter_mut()
            .zip(&outputs.outputs)
        {
            value.ty = expected.ty.clone();
        }
        match builder.block_outputs(&typed_body, &outputs.outputs) {
            Ok(_) => {
                if let Err(error) = builder.register_boundary_signature(
                    declaration.signature,
                    declaration.captures,
                    global_bindings,
                ) {
                    errors.push(error);
                    continue;
                }
                if let Err(error) = owner_lifetime::register(&mut builder.cfg) {
                    errors.push(error);
                    continue;
                }
                builder.publish_rebind_associations();
                builder.cfg.initial_points = builder.cfg.points.clone();
                if let Err(error) = name_resolution::validate_literal_source(&builder.cfg, parent, &declaration, global_bindings, &builder.value_types.expressions) { errors.push(error); continue; }
                if let Err(error) = name_resolution::validate(&builder.cfg, structs) { errors.push(error); continue; }
                if let Err(error) =
                    selected_source::register_literal_members(&mut builder.cfg, structs, &builder.generic_scope, &builder.generic_declarations)
                {
                    errors.push(error);
                    continue;
                }
                let capture_input = match source_boundary::local_preflight_input_with_ancestor(&builder.cfg, structs, enums, generic_parameters, generic_declarations, ancestor_input) { Ok(input) => input, Err(error) => { errors.push(error); continue; } };
                let initial_transfer = transfer_reference_facts_observed(
                    &mut builder.cfg,
                    &BTreeMap::new(),
                    &[],
                    &[],
                    &[],
                    &capture_input,
                    #[cfg(test)]
                    ReferenceTransferTestRole::LiteralRefresh,
                    #[cfg(test)]
                    ReferenceTransferTestStage::InitialConstruction,
                    #[cfg(test)]
                    trace,
                );
                errors.extend(record_literal_reference_cfgs(
                    &mut builder.cfg,
                    &capture_input,
                    &initial_transfer,
                    source,
                    assignment_context_span,
                    generic_scope,
                    generic_declarations,
                    generic_parameters,
                    module_names,
                    global_bindings,
                    call_sites,
                    structs,
                    enums,
                    assignment_outputs,
                    literal_signatures,
                    value_types,
                    #[cfg(test)]
                    trace,
                ));
                if !errors.is_empty() { continue; }
                let completion = source_boundary::local_preflight_input_with_ancestor(&builder.cfg, structs, enums, generic_parameters, generic_declarations, ancestor_input)
                    .and_then(|input| source_boundary::complete_capture_children(&mut builder.cfg, &input)
                        .and_then(|()| source_boundary::seal_capture_tree(&mut builder.cfg, &input.policy_context)));
                if let Err(error) = completion { errors.push(error); continue; }
                literal_cfgs.push(ScalarLiteralReferenceCfg {
                    constructor_seed,
                    literal: declaration.literal_span.clone(),
                    capture_point: declaration.event.point,
                    captures: declaration.captures.to_vec(),
                    reference_cfg: Box::new(builder.cfg),
                });
            }
            Err(error) => errors.push(error),
        }
    }
    name_resolution::literal_seed::publish(parent, literal_cfgs, errors)
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
    project_block_occurrences(&mut selected, position);
    selected
}

fn project_block_occurrences(block: &mut ScalarBlock, position: usize) {
    block.occurrence.output_projection.insert(0, position);
    for item in &mut block.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                binding.occurrence.output_projection.insert(0, position);
                project_while_occurrences_in_expression(&mut binding.value, position);
                for output in &mut binding.output_values {
                    project_while_occurrences_in_expression(&mut output.value, position);
                }
                for receiver in &mut binding.receivers {
                    if let Some(length) = &mut receiver.allocation_length {
                        project_while_occurrences_in_expression(length, position);
                    }
                }
            }
            ScalarBlockItem::Expression(expression) => {
                project_while_occurrences_in_expression(expression, position);
            }
            ScalarBlockItem::Assignment(assignment) => {
                assignment.occurrence.output_projection.insert(0, position);
                for value in &mut assignment.values {
                    project_while_occurrences_in_expression(value, position);
                }
                for target in &mut assignment.targets {
                    project_while_occurrences_in_place(&mut target.place, position);
                }
            }
            ScalarBlockItem::While(while_expression) => {
                while_expression
                    .occurrence
                    .output_projection
                    .insert(0, position);
                project_while_occurrences_in_expression(&mut while_expression.condition, position);
                project_block_occurrences(&mut while_expression.body, position);
            }
        }
    }
    for expression in &mut block.expressions {
        project_while_occurrences_in_expression(expression, position);
    }
    for output in &mut block.final_output_values {
        project_while_occurrences_in_expression(&mut output.value, position);
    }
}

fn project_while_occurrences_in_place(place: &mut ScalarPlace, position: usize) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Field { base, .. } => project_while_occurrences_in_place(base, position),
        ScalarPlace::Dereference { pointer, .. } => {
            project_while_occurrences_in_expression(pointer, position);
        }
        ScalarPlace::Index { base, index, .. } => {
            project_while_occurrences_in_place(base, position);
            project_while_occurrences_in_expression(index, position);
        }
    }
}

fn project_while_occurrences_in_expression(expression: &mut ScalarExpression, position: usize) {
    match expression {
        ScalarExpression::Assignment(assignment) => {
            assignment.occurrence.output_projection.insert(0, position);
            for target in &mut assignment.targets {
                project_while_occurrences_in_place(&mut target.place, position);
            }
            for value in &mut assignment.values {
                project_while_occurrences_in_expression(value, position);
            }
        }
        ScalarExpression::FunctionLiteral { body, .. } | ScalarExpression::Block(body) => {
            project_block_occurrences(body, position)
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            project_while_occurrences_in_expression(condition, position);
            project_block_occurrences(then_branch, position);
            project_block_occurrences(else_branch, position);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            project_while_occurrences_in_expression(condition, position);
            project_block_occurrences(then_branch, position);
        }
        ScalarExpression::Binary { left, right, .. } => {
            project_while_occurrences_in_expression(left, position);
            project_while_occurrences_in_expression(right, position);
        }
        ScalarExpression::Unary { operand, .. } => {
            project_while_occurrences_in_expression(operand, position);
        }
        ScalarExpression::Call {
            arguments,
            occurrence,
            ..
        } => {
            occurrence.output_projection.insert(0, position);
            for argument in arguments {
                project_while_occurrences_in_expression(argument, position);
            }
        }
        ScalarExpression::ArrayLiteral {
            elements: arguments,
            ..
        } => {
            for argument in arguments {
                project_while_occurrences_in_expression(argument, position);
            }
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                project_while_occurrences_in_expression(&mut field.value, position);
            }
        }
        ScalarExpression::Dereference {
            place, occurrence, ..
        }
        | ScalarExpression::IndexedRead {
            place, occurrence, ..
        } => {
            occurrence.output_projection.insert(0, position);
            project_while_occurrences_in_place(place, position);
        }
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. } => {
            project_while_occurrences_in_place(place, position);
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

#[cfg(test)]
#[path = "c2_capture_construction_tests.rs"]
mod c2_capture_construction_tests;

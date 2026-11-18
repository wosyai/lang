use super::control::{
    module_initialization_region, BindingOutput, BindingRegion, BlockItem, BlockItemKind,
    BlockPosition, BlockRegion, ConditionalRegion, FinalOutputSlot, ModuleInitializationRegion,
    ShortCircuitRegion, WhileRegion,
};
use super::control::{
    AssignmentEvaluation, AssignmentGroup, AssignmentProjection, AssignmentReplacement,
    AssignmentSlot,
};
#[cfg(test)]
use super::control::{ModuleInitializationStepKind, NamespaceTarget};
use super::*;

#[cfg(test)]
mod input_tests;

#[cfg(test)]
pub(in crate::scalar) mod stage_observations {
    use super::*;

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub(in crate::scalar) struct Counts {
        pub call_sites: usize,
        pub reference_origins: usize,
        pub output_contracts: usize,
        pub copy_specialization: usize,
    }

    thread_local! {
        static ACTIVE: RefCell<Option<Counts>> = const { RefCell::new(None) };
    }

    pub(in crate::scalar) struct Scope {
        previous: Option<Counts>,
    }

    impl Scope {
        pub(in crate::scalar) fn new() -> Self {
            Self {
                previous: ACTIVE.with(|active| active.replace(Some(Counts::default()))),
            }
        }

        pub(in crate::scalar) fn counts(&self) -> Counts {
            ACTIVE.with(|active| {
                *active
                    .borrow()
                    .as_ref()
                    .expect("active reference-stage observations")
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

    pub(super) fn record_call_sites() {
        record(|counts| counts.call_sites += 1);
    }
    pub(super) fn record_reference_origins() {
        record(|counts| counts.reference_origins += 1);
    }
    pub(super) fn record_output_contracts() {
        record(|counts| counts.output_contracts += 1);
    }
    pub(super) fn record_copy_specialization() {
        record(|counts| counts.copy_specialization += 1);
    }
}

#[cfg(test)]
pub(super) fn record_program_reference_origins(
    program: &mut ScalarProgram,
    call_sites: &ModuleCallSites,
) -> Vec<ReferenceAnalysisError> {
    analyze_program_references(program, call_sites).install(program)
}

pub(super) struct ProgramReferenceResults {
    updates: Vec<(usize, FunctionReferenceResult)>,
    errors: Vec<ReferenceAnalysisError>,
}

impl ProgramReferenceResults {
    pub(super) fn install(self, program: &mut ScalarProgram) -> Vec<ReferenceAnalysisError> {
        install_module_reference_results(&mut program.items, self.updates);
        self.errors
    }
}

pub(super) fn analyze_program_references(
    program: &ScalarProgram,
    call_sites: &ModuleCallSites,
) -> ProgramReferenceResults {
    let (mut updates, mut errors) = derive_and_converge_program_references(program, call_sites);
    if errors.is_empty() {
        errors.extend(specialize_reference_results(
            &mut [ModuleReferenceView {
                source: &program.source,
                items: &program.items,
                results: &mut updates,
            }],
            &program.structs,
            &program.enums,
        ));
    }
    ProgramReferenceResults { updates, errors }
}

fn derive_and_converge_program_references(
    program: &ScalarProgram,
    call_sites: &ModuleCallSites,
) -> (
    Vec<(usize, FunctionReferenceResult)>,
    Vec<ReferenceAnalysisError>,
) {
    let (mut updates, mut errors) = derive_module_reference_origins(
        &program.source,
        &program.items,
        call_sites,
        &program.structs,
        &program.enums,
    );
    if errors.is_empty() {
        errors.extend(resolve_reference_output_contracts(&mut [
            ModuleReferenceView {
                source: &program.source,
                items: &program.items,
                results: &mut updates,
            },
        ]));
    }
    (updates, errors)
}

pub(super) struct ReferenceFlowBuilder<'a> {
    pub(super) source: SourceIdentity,
    pub(super) function_span: ByteSpan,
    pub(super) names: BTreeMap<String, ReferenceBindingId>,
    pub(super) module_names: &'a HashSet<String>,
    pub(super) call_sites: &'a FunctionCallSites,
    pub(super) origins: HashMap<ReferenceBindingId, BTreeSet<ReferenceOriginId>>,
    pub(super) checked: HashSet<ReferenceBindingId>,
    pub(super) binding_types: HashMap<ReferenceBindingId, ScalarType>,
    pub(super) structs: &'a [ScalarStruct],
    pub(super) enums: &'a [ScalarEnum],
    pub(super) generic_parameters: BTreeSet<String>,
    pub(super) dereference_points: HashMap<OccurrenceOrdinal, CfgPointId>,
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

enum ReferenceAssignmentDestination {
    Name,
    EvaluationOnly,
    Through(CfgPointId),
    Resolved(ReferencePlaceId),
}

#[derive(Clone, Copy)]
enum CheckedReadSource {
    Place(OccurrenceOrdinal),
    SyntheticMember,
}

struct ReferencePlaceRoot<'a> {
    place: &'a ScalarPlace,
    ordinal: OccurrenceOrdinal,
}

impl<'a> ReferenceFlowBuilder<'a> {
    pub(super) fn new(
        source: &SourceIdentity,
        function: &ScalarFunction,
        module_names: &'a HashSet<String>,
        call_sites: &'a FunctionCallSites,
        structs: &'a [ScalarStruct],
        enums: &'a [ScalarEnum],
    ) -> Self {
        Self {
            source: source.clone(),
            function_span: function.span,
            names: BTreeMap::new(),
            module_names,
            call_sites,
            origins: HashMap::new(),
            checked: HashSet::new(),
            binding_types: HashMap::new(),
            structs,
            enums,
            generic_parameters: call_sites
                .boundary
                .generics
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect(),
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
        }
    }

    fn assignment_output_types(
        &self,
        group: &AssignmentGroup,
    ) -> Result<Vec<ScalarType>, ReferenceAnalysisError> {
        group
            .destinations()
            .map(|slot| match &slot.replacement {
                AssignmentReplacement::Resolved(ty) => Ok(ty.clone()),
                AssignmentReplacement::Missing => Err(self.missing_context(
                    slot.target_span,
                    ReferenceAnalysisErrorReason::MissingResolvedBinding,
                )),
            })
            .collect()
    }

    fn call_target(
        &self,
        expression: &ScalarExpression,
        ordinal: OccurrenceOrdinal,
    ) -> Result<ResolvedCallTarget, ReferenceAnalysisError> {
        let span = expression_span(expression);
        self.call_sites
            .call_occurrences
            .binary_search_by_key(&ordinal.0, |association| association.ordinal.0)
            .ok()
            .and_then(|index| {
                self.call_sites
                    .sites
                    .get(self.call_sites.call_occurrences[index].site)
            })
            .map(|site| site.target.clone())
            .ok_or_else(|| {
                self.missing_context(
                    span,
                    ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                )
            })
    }

    fn structural_child(&self, parent: OccurrenceParent, edge: OccurrenceEdge) -> OccurrenceNode {
        *self
            .call_sites
            .structure
            .edges
            .get(&(parent, edge))
            .unwrap_or_else(|| {
                panic!(
                    "missing structural child {parent:?}/{edge:?} in {:?} callable {:?}",
                    self.source, self.function_span
                )
            })
    }

    fn expression_ordinal(
        &self,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
    ) -> OccurrenceOrdinal {
        let OccurrenceNode::Expression(ordinal) = self.structural_child(parent, edge) else {
            panic!(
                "expected expression child {parent:?}/{edge:?} in {:?}",
                self.source
            )
        };
        ordinal
    }

    fn place_ordinal(&self, parent: OccurrenceParent, edge: OccurrenceEdge) -> OccurrenceOrdinal {
        let OccurrenceNode::Place(ordinal) = self.structural_child(parent, edge) else {
            panic!(
                "expected place child {parent:?}/{edge:?} in {:?}",
                self.source
            )
        };
        ordinal
    }

    fn block_ordinal(&self, parent: OccurrenceParent, edge: OccurrenceEdge) -> BlockOrdinal {
        let OccurrenceNode::Block(ordinal) = self.structural_child(parent, edge) else {
            panic!(
                "expected block child {parent:?}/{edge:?} in {:?}",
                self.source
            )
        };
        ordinal
    }

    fn expression_child(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let ordinal = self.expression_ordinal(parent, edge);
        self.expression(expression, scope, ordinal)
    }

    fn place_root<'p>(
        &self,
        mut place: &'p ScalarPlace,
        mut ordinal: OccurrenceOrdinal,
    ) -> ReferencePlaceRoot<'p> {
        while let ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } = place {
            ordinal =
                self.place_ordinal(OccurrenceParent::Place(ordinal), OccurrenceEdge::PlaceBase);
            place = base;
        }
        ReferencePlaceRoot { place, ordinal }
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
        region: &ConditionalRegion,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let (condition_ordinal, then_block) = match region {
            ConditionalRegion::Value {
                condition,
                then_block,
                ..
            } => (*condition, *then_block),
            ConditionalRegion::Unit {
                condition,
                then_block,
                ..
            } => (*condition, *then_block),
        };
        self.expression(condition, scope, condition_ordinal)?;
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
        let then_value = self.block(then_branch, then_block)?;
        let then_end = self.cursor.expect("then branch terminal");
        let then_checked = self.checked.clone();

        self.names = incoming_names;
        self.origins = incoming_origins.clone();
        self.checked = incoming_checked.clone();
        self.cursor = Some(branch);
        let (else_value, else_end) = if let Some(else_branch) = else_branch {
            let ConditionalRegion::Value { else_block, .. } = region else {
                unreachable!("value conditional region")
            };
            let result = self.block(else_branch, *else_block)?;
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
        ordinal: OccurrenceOrdinal,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let span = expression_span(expression);
        let parent = OccurrenceParent::Expression(ordinal);
        let calls = self.call_sites;
        match expression {
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let region = &calls.conditionals[&ordinal];
                assert!(
                    matches!(region, ConditionalRegion::Value { .. }),
                    "expected value conditional at {ordinal:?} in {:?}",
                    self.source
                );
                self.conditional(
                    condition,
                    then_branch,
                    Some(else_branch),
                    span,
                    scope,
                    region,
                )
            }
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                let region = &calls.conditionals[&ordinal];
                assert!(
                    matches!(region, ConditionalRegion::Unit { .. }),
                    "expected unit conditional at {ordinal:?} in {:?}",
                    self.source
                );
                self.conditional(condition, then_branch, None, span, scope, region)
            }
            ScalarExpression::Block(block) => match self
                .block(block, self.block_ordinal(parent, OccurrenceEdge::BlockBody))?
            {
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
                let target = self.call_target(expression, ordinal)?;
                let mut address = None;
                let mut checked_release_address = None;
                let mut argument_points = Vec::new();
                for (position, argument) in arguments.iter().enumerate() {
                    let ordinal =
                        self.expression_ordinal(parent, OccurrenceEdge::CallArgument(position));
                    self.expression(argument, scope, ordinal)?;
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
                    self.move_value(argument, scope, ordinal)?;
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
            ScalarExpression::Binary {
                operator,
                left,
                right,
                ..
            } => {
                if matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
                    let region = &calls.short_circuits[&ordinal];
                    self.expression(left, scope, region.left)?;
                    self.expression(right, scope, region.right)?;
                } else {
                    self.expression_child(left, scope, parent, OccurrenceEdge::BinaryLeft)?;
                    self.expression_child(right, scope, parent, OccurrenceEdge::BinaryRight)?;
                }
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Unary { operand, .. } => {
                self.expression_child(operand, scope, parent, OccurrenceEdge::UnaryOperand)?;
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for (position, element) in elements.iter().enumerate() {
                    let ordinal =
                        self.expression_ordinal(parent, OccurrenceEdge::AggregateElement(position));
                    self.expression(element, scope, ordinal)?;
                    self.move_value(element, scope, ordinal)?;
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
                for (position, field) in fields.iter().enumerate() {
                    let ordinal =
                        self.expression_ordinal(parent, OccurrenceEdge::AggregateField(position));
                    self.expression(&field.value, scope, ordinal)?;
                    self.move_value(&field.value, scope, ordinal)?;
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
            | ScalarExpression::IndexedRead { place, .. } => self.checked_place_read(
                place,
                expression,
                span,
                scope,
                CheckedReadSource::Place(
                    self.place_ordinal(parent, OccurrenceEdge::ExpressionPlace),
                ),
            ),
            ScalarExpression::RawAddress { place, .. } => {
                self.place_expressions(
                    place,
                    scope,
                    self.place_ordinal(parent, OccurrenceEdge::ExpressionPlace),
                )?;
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
                self.checked_place_read(
                    &place,
                    expression,
                    span,
                    scope,
                    CheckedReadSource::SyntheticMember,
                )
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
                let place_ordinal = self.place_ordinal(parent, OccurrenceEdge::ExpressionPlace);
                let root = self.place_root(place, place_ordinal);
                let parents = match root.place {
                    ScalarPlace::Dereference { pointer, .. } => {
                        let ReferenceValue::Checked(origins) = self.expression_child(
                            pointer,
                            scope,
                            OccurrenceParent::Place(root.ordinal),
                            OccurrenceEdge::DereferencePointer,
                        )?
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
                self.place_indices(place, scope, place_ordinal)?;
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
                        match (root.place, &parent) {
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
        ordinal: OccurrenceOrdinal,
    ) -> Result<(), ReferenceAnalysisError> {
        let parent = OccurrenceParent::Place(ordinal);
        match place {
            ScalarPlace::Name { name, span } => {
                self.place_name(name, *span, scope)?;
            }
            ScalarPlace::Dereference { pointer, .. } => {
                self.expression_child(pointer, scope, parent, OccurrenceEdge::DereferencePointer)?;
                let ScalarPlace::Dereference { span, .. } = place else {
                    unreachable!()
                };
                let point = self.cursor.ok_or_else(|| {
                    self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    )
                })?;
                self.dereference_points.insert(ordinal, point);
            }
            ScalarPlace::Field { base, .. } => self.place_expressions(
                base,
                scope,
                self.place_ordinal(parent, OccurrenceEdge::PlaceBase),
            )?,
            ScalarPlace::Index { base, index, .. } => {
                self.place_expressions(
                    base,
                    scope,
                    self.place_ordinal(parent, OccurrenceEdge::PlaceBase),
                )?;
                self.expression_child(index, scope, parent, OccurrenceEdge::PlaceIndex)?;
            }
        }
        Ok(())
    }

    fn place_name(
        &mut self,
        name: &str,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        if self.names.contains_key(name) && self.checked.contains(&self.binding(name, span)?) {
            let binding = self.binding(name, span)?;
            self.point(CfgPointKind::Read { binding }, span, scope, BTreeSet::new());
        } else if !self.names.contains_key(name) && !self.module_names.contains(name) {
            return Err(
                self.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
            );
        }
        Ok(())
    }

    fn read_place_expressions(
        &mut self,
        place: &ScalarPlace,
        scope: ReferenceScopeId,
        source: CheckedReadSource,
    ) -> Result<(), ReferenceAnalysisError> {
        match source {
            CheckedReadSource::Place(ordinal) => self.place_expressions(place, scope, ordinal),
            CheckedReadSource::SyntheticMember => {
                let ScalarPlace::Field { base, .. } = place else {
                    unreachable!("synthetic member field")
                };
                let ScalarPlace::Name { name, span } = base.as_ref() else {
                    unreachable!("synthetic member receiver")
                };
                self.place_name(name, *span, scope)
            }
        }
    }

    fn checked_place_read(
        &mut self,
        place: &ScalarPlace,
        expression: &ScalarExpression,
        span: ByteSpan,
        scope: ReferenceScopeId,
        source: CheckedReadSource,
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
            self.read_place_expressions(place, scope, source)?;
            self.point(
                CfgPointKind::Evaluate(expression.clone()),
                span,
                scope,
                BTreeSet::new(),
            );
            return Ok(ReferenceValue::Other);
        }
        self.read_place_expressions(place, scope, source)?;
        let pointer_point = match source {
            CheckedReadSource::SyntheticMember => None,
            CheckedReadSource::Place(ordinal) => {
                let root = self.place_root(place, ordinal);
                match root.place {
                    ScalarPlace::Dereference { span, .. } => {
                        Some(*self.dereference_points.get(&root.ordinal).ok_or_else(|| {
                            self.missing_context(
                                *span,
                                ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            )
                        })?)
                    }
                    ScalarPlace::Name { .. } => None,
                    ScalarPlace::Field { .. } | ScalarPlace::Index { .. } => unreachable!(),
                }
            }
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
        ordinal: OccurrenceOrdinal,
    ) -> Result<(), ReferenceAnalysisError> {
        let parent = OccurrenceParent::Place(ordinal);
        match place {
            ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => {}
            ScalarPlace::Field { base, .. } => self.place_indices(
                base,
                scope,
                self.place_ordinal(parent, OccurrenceEdge::PlaceBase),
            )?,
            ScalarPlace::Index { base, index, .. } => {
                self.place_indices(
                    base,
                    scope,
                    self.place_ordinal(parent, OccurrenceEdge::PlaceBase),
                )?;
                self.expression_child(index, scope, parent, OccurrenceEdge::PlaceIndex)?;
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
        ordinal: OccurrenceOrdinal,
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
                let policy = copy_policy(&ty, self.structs, self.enums, &self.generic_parameters)
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
                let root = self.place_root(
                    place,
                    self.place_ordinal(
                        OccurrenceParent::Expression(ordinal),
                        OccurrenceEdge::ExpressionPlace,
                    ),
                );
                let ScalarPlace::Dereference { pointer, .. } = root.place else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                let ty = self.indexed_move_type(place)?;
                let policy = copy_policy(&ty, self.structs, self.enums, &self.generic_parameters)
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
                let pointer_point =
                    *self.dereference_points.get(&root.ordinal).ok_or_else(|| {
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
        let policy =
            copy_policy(&ty, self.structs, self.enums, &self.generic_parameters).map_err(|_| {
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

    fn project_call_output(&mut self, producer: CfgPointId, output: usize) -> CfgPointId {
        let point = &self.cfg.points[producer.0];
        let CfgPointKind::Call {
            expression,
            output: 0,
            target,
            argument_points,
        } = &point.kind
        else {
            unreachable!("evaluated output-zero call")
        };
        let kind = CfgPointKind::Call {
            expression: expression.clone(),
            output,
            target: target.clone(),
            argument_points: argument_points.clone(),
        };
        self.point_with_origins(kind, point.source_span.range, point.scope, None)
    }

    fn additional_receivers(
        &mut self,
        binding: &ScalarBinding,
        block: &ScalarBlock,
        scope: ReferenceScopeId,
        declared: &mut Vec<(String, ReferenceBindingId, Option<ReferenceBindingId>)>,
        call_producer: Option<CfgPointId>,
        region: &BindingRegion,
    ) -> Result<(), ReferenceAnalysisError> {
        for receiver in &region.receivers()[1..] {
            match &region.secondary[receiver.position - 1] {
                BindingOutput::Evaluate {
                    position: source,
                    expression,
                } => {
                    self.expression(&binding.output_values[*source].value, scope, *expression)?;
                }
                BindingOutput::ProjectPrimary { output } => {
                    if let Some(producer) = call_producer {
                        self.project_call_output(producer, *output);
                    }
                }
            }
            let id = ReferenceBindingId {
                source: self.source.clone(),
                function_span: self.function_span,
                declaration_span: receiver.span,
                block_span: block.span,
                kind: ReferenceBindingKind::Declared,
            };
            let previous = self.names.insert(receiver.name.clone(), id.clone());
            self.binding_types.insert(id.clone(), receiver.ty.clone());
            declared.push((receiver.name.clone(), id.clone(), previous));
            if matches!(receiver.ty, ScalarType::CheckedReference { .. }) {
                self.checked.insert(id.clone());
                self.uncomputed_point(CfgPointKind::Bind { binding: id }, receiver.span, scope);
            } else {
                self.point(
                    CfgPointKind::Bind { binding: id },
                    receiver.span,
                    scope,
                    BTreeSet::new(),
                );
            }
        }
        Ok(())
    }

    fn assignment_destination(
        &mut self,
        target: &ScalarAssignmentTarget,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
    ) -> Result<ReferenceAssignmentDestination, ReferenceAnalysisError> {
        if matches!(target.place, ScalarPlace::Name { .. }) {
            return Ok(ReferenceAssignmentDestination::Name);
        }
        if let ScalarPlace::Name { name, span } = reference_root(&target.place) {
            if self.module_names.contains(name) && !self.names.contains_key(name) {
                self.point(
                    CfgPointKind::Evaluate(ScalarExpression::Name {
                        name: name.clone(),
                        span: *span,
                    }),
                    target.target_span,
                    scope,
                    BTreeSet::new(),
                );
                return Ok(ReferenceAssignmentDestination::EvaluationOnly);
            }
        }
        self.place_expressions(&target.place, scope, ordinal)?;
        let root = self.place_root(&target.place, ordinal);
        if let ScalarPlace::Dereference { .. } = root.place {
            let pointer_point = *self.dereference_points.get(&root.ordinal).ok_or_else(|| {
                self.missing_context(
                    target.target_span,
                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                )
            })?;
            Ok(ReferenceAssignmentDestination::Through(pointer_point))
        } else {
            Ok(ReferenceAssignmentDestination::Resolved(
                self.assignment_place(&target.place, target.target_span)?,
            ))
        }
    }

    fn block(
        &mut self,
        block: &ScalarBlock,
        ordinal: BlockOrdinal,
    ) -> Result<Option<ReferenceValue>, ReferenceAnalysisError> {
        let scope = ReferenceScopeId(self.next_scope);
        self.next_scope += 1;
        let calls = self.call_sites;
        let region = &calls.structure.blocks[ordinal.0];
        assert_eq!(
            region.items.len(),
            block.items.len(),
            "mismatched block item count in {:?}",
            self.source
        );
        self.point(
            CfgPointKind::ScopeEnter,
            region.span,
            scope,
            BTreeSet::new(),
        );
        let mut declared = Vec::new();
        let mut result = None;
        let mut return_values = Vec::new();
        let mut next_item = 0;
        while next_item < region.items.len() {
            let record = &region.items[next_item];
            assert_eq!(
                record.position, next_item,
                "mismatched block item position in {:?}",
                self.source
            );
            let item_index = record.position;
            next_item = item_index + 1;
            let item = &block.items[item_index];
            match (item, &record.kind) {
                (
                    ScalarBlockItem::LocalBinding(binding),
                    BlockItemKind::Binding(binding_region),
                ) => {
                    let receiver = &binding_region.receivers()[0];
                    let primary = &binding.value;
                    let primary_ordinal = binding_region.primary;
                    let value = if matches!(receiver.ty, ScalarType::RawPointer(_))
                        && matches!(primary, ScalarExpression::Name { name, .. } if name == "null")
                    {
                        self.point(
                            CfgPointKind::Evaluate(primary.clone()),
                            expression_span(primary),
                            scope,
                            BTreeSet::new(),
                        );
                        ReferenceValue::Other
                    } else {
                        self.expression(primary, scope, primary_ordinal)?
                    };
                    let call_producer = if binding.output_origin
                        == ScalarBindingOutputOrigin::SingleExpression
                        && matches!(primary, ScalarExpression::Call { .. })
                    {
                        Some(self.cursor.expect("evaluated binding call"))
                    } else {
                        None
                    };
                    self.move_value(primary, scope, primary_ordinal)?;
                    let id = ReferenceBindingId {
                        source: self.source.clone(),
                        function_span: self.function_span,
                        declaration_span: receiver.span,
                        block_span: block.span,
                        kind: ReferenceBindingKind::Declared,
                    };
                    self.binding_types.insert(id.clone(), receiver.ty.clone());
                    if binding.is_allocation {
                        self.cfg.allocations.insert(id.clone());
                    }
                    let origins = match value {
                        ReferenceValue::Checked(origins) => Some(origins),
                        ReferenceValue::Other
                            if matches!(receiver.ty, ScalarType::CheckedReference { .. }) =>
                        {
                            return Err(self.missing_origin(
                                receiver.span,
                                id,
                                ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            ));
                        }
                        ReferenceValue::Other => None,
                        ReferenceValue::Uncomputed
                            if matches!(receiver.ty, ScalarType::CheckedReference { .. }) =>
                        {
                            self.checked.insert(id.clone());
                            self.origins.remove(&id);
                            self.uncomputed_point(
                                CfgPointKind::Bind {
                                    binding: id.clone(),
                                },
                                receiver.span,
                                scope,
                            );
                            let old = self.names.insert(receiver.name.clone(), id.clone());
                            declared.push((receiver.name.clone(), id, old));
                            self.additional_receivers(
                                binding,
                                block,
                                scope,
                                &mut declared,
                                call_producer,
                                binding_region,
                            )?;
                            continue;
                        }
                        ReferenceValue::Uncomputed => None,
                    };
                    let old = self.names.insert(receiver.name.clone(), id.clone());
                    if let Some(origins) = origins {
                        self.checked.insert(id.clone());
                        self.origins.insert(id.clone(), origins.clone());
                        self.point(
                            CfgPointKind::Bind {
                                binding: id.clone(),
                            },
                            receiver.span,
                            scope,
                            origins,
                        );
                    } else {
                        self.point(
                            CfgPointKind::Bind {
                                binding: id.clone(),
                            },
                            receiver.span,
                            scope,
                            BTreeSet::new(),
                        );
                    }
                    declared.push((receiver.name.clone(), id, old));
                    self.additional_receivers(
                        binding,
                        block,
                        scope,
                        &mut declared,
                        call_producer,
                        binding_region,
                    )?;
                }
                (
                    ScalarBlockItem::Expression(expression),
                    kind @ (BlockItemKind::Evaluation { .. } | BlockItemKind::FinalOutput(_)),
                ) => {
                    let ordinal = match kind {
                        BlockItemKind::Evaluation { expression } => *expression,
                        BlockItemKind::FinalOutput(slot) => slot.expression,
                        _ => unreachable!("expression item region"),
                    };
                    let value = self.expression(expression, scope, ordinal)?;
                    let value_point = self.cursor.expect("evaluated output expression");
                    self.move_value(expression, scope, ordinal)?;
                    if let BlockItemKind::FinalOutput(slot) = kind {
                        let ty = if scope == ReferenceScopeId(0) {
                            &calls.boundary.root_outputs[slot.output].ty
                        } else {
                            &block.final_output_values[slot.output].ty
                        };
                        return_values.push(value_point);
                        if matches!(value, ReferenceValue::Checked(_)) {
                            result = Some(value);
                        } else if matches!(value, ReferenceValue::Uncomputed)
                            && matches!(ty, ScalarType::CheckedReference { .. })
                        {
                            result = Some(value);
                        }
                    }
                }
                (
                    ScalarBlockItem::Assignment(assignment),
                    BlockItemKind::Assignment { group: key },
                ) => {
                    let calls = self.call_sites;
                    let group = &calls.assignments[&(key.block, key.item)];
                    let output_types = self.assignment_output_types(group)?;
                    let mut values = Vec::new();
                    for evaluation in group.rhs_evaluations() {
                        let value = &assignment.values[evaluation.position];
                        let result = self.expression(value, scope, evaluation.expression)?;
                        let rhs_point = self.cursor.expect("evaluated assignment output");
                        self.move_value(value, scope, evaluation.expression)?;
                        values.push((result, rhs_point));
                    }
                    for projection in group.projected_outputs() {
                        let producer = values[projection.producer.position].1;
                        let rhs_point = self.project_call_output(producer, projection.output);
                        values.push((ReferenceValue::Uncomputed, rhs_point));
                    }
                    let destinations = group
                        .destinations()
                        .zip(&values)
                        .map(|(slot, _)| {
                            self.assignment_destination(
                                &assignment.targets[slot.position],
                                scope,
                                slot.destination,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let capture = if group.previous_observations().len() > 1 {
                        Some(self.point(
                            CfgPointKind::CaptureAssignmentPrevious { writes: Vec::new() },
                            group.span,
                            scope,
                            BTreeSet::new(),
                        ))
                    } else {
                        None
                    };
                    for ((slot, (value, rhs_point)), destination) in
                        group.installs().zip(values).zip(destinations)
                    {
                        let position = slot.position;
                        let target = &assignment.targets[position];
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
                            if let ReferenceAssignmentDestination::Through(pointer_point) =
                                destination
                            {
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
                                let target_place = match destination {
                                    ReferenceAssignmentDestination::EvaluationOnly => continue,
                                    ReferenceAssignmentDestination::Resolved(place) => place,
                                    ReferenceAssignmentDestination::Name
                                    | ReferenceAssignmentDestination::Through(_) => {
                                        unreachable!("staged projected assignment destination")
                                    }
                                };
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
                        if let Some(capture) = capture {
                            let install = self.cursor.expect("assignment install point");
                            let CfgPointKind::CaptureAssignmentPrevious { writes } =
                                &mut self.cfg.points[capture.0].kind
                            else {
                                unreachable!("assignment group capture")
                            };
                            writes.push(install);
                        }
                    }
                    let continuation = group.continuation();
                    assert_eq!(
                        continuation,
                        BlockPosition {
                            block: ordinal,
                            item: item_index + 1
                        }
                    );
                    next_item = continuation.item;
                }
                (ScalarBlockItem::While(loop_), BlockItemKind::While { region: key }) => {
                    let loop_region = self.call_sites.loops[&(key.block, key.item)];
                    let (body, test) = loop_region.body_to_test();
                    let loop_head = self.point(
                        CfgPointKind::LoopTest {
                            condition: loop_.condition.clone(),
                        },
                        expression_span(&loop_.condition),
                        scope,
                        BTreeSet::new(),
                    );
                    self.expression(&loop_.condition, scope, test)?;
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
                    self.block(&loop_.body, body)?;
                    let back = self.cursor.expect("loop body terminal");
                    self.cfg.points[back.0].successors.push(loop_head);
                    self.names = names;
                    self.origins = origins;
                    self.checked = checked;
                    self.cursor = Some(branch);
                    self.point(CfgPointKind::Join, loop_.span, scope, BTreeSet::new());
                    assert_eq!(
                        loop_region.on_false,
                        BlockPosition {
                            block: ordinal,
                            item: item_index + 1
                        }
                    );
                    next_item = loop_region.on_false.item;
                }
                _ => panic!(
                    "mismatched block item {ordinal:?}/{item_index} in {:?}",
                    self.source
                ),
            }
        }
        if scope == ReferenceScopeId(0) {
            for (slot, value_point) in region.final_outputs().zip(return_values) {
                let metadata = &calls.boundary.root_outputs[slot.output];
                assert_eq!(metadata.output, slot.output);
                self.uncomputed_point(
                    CfgPointKind::ReturnOutput {
                        output: slot.output,
                        value_point,
                        checked: matches!(metadata.ty, ScalarType::CheckedReference { .. }),
                    },
                    slot.span,
                    scope,
                );
            }
        }
        self.point(CfgPointKind::ScopeExit, region.span, scope, BTreeSet::new());
        if scope == ReferenceScopeId(0) && region.final_outputs().next().is_some() {
            let return_span = ByteSpan::new(
                region.final_outputs().next().expect("output").span.start,
                region.final_outputs().last().expect("output").span.end,
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
    assignment_previous: BTreeMap<CfgPointId, BTreeSet<ReferenceOriginId>>,
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

fn assignment_pointer_candidates(
    place: &ScalarPlace,
    origins: &BTreeSet<ReferenceOriginId>,
    facts: &[ReferenceOrigin],
) -> Vec<InvalidationCandidate> {
    origins
        .iter()
        .filter_map(|origin| {
            let loan = match &facts[origin.0] {
                ReferenceOrigin::Fresh { loan, .. }
                | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => return None,
            };
            let mut resolved_place = loan.origin_place.clone();
            resolved_place.place = place.clone();
            collect_reference_projections(place, None, &mut resolved_place.projections);
            Some(InvalidationCandidate {
                origin: *origin,
                place: resolved_place,
                loan: loan.clone(),
            })
        })
        .collect()
}

fn previous_place_origins(
    state: &ReferenceEdgeState,
    facts: &[ReferenceOrigin],
    target: &ReferencePlaceId,
) -> BTreeSet<ReferenceOriginId> {
    if target.projections.is_empty() {
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
            .filter(|origin| match &facts[origin.0] {
                ReferenceOrigin::Fresh { loan, .. }
                | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                    reference_places_overlap(&loan.origin_place, target)
                }
                ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
            })
            .collect()
    }
}

fn previous_candidate_origins(
    state: &ReferenceEdgeState,
    facts: &[ReferenceOrigin],
    candidates: &[InvalidationCandidate],
) -> BTreeSet<ReferenceOriginId> {
    state
        .bindings
        .values()
        .flat_map(|origins| origins.iter().copied())
        .filter(|origin| match &facts[origin.0] {
            ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                !candidates
                    .iter()
                    .any(|candidate| candidate.origin == *origin)
                    && candidates.iter().any(|candidate| {
                        reference_places_overlap(&loan.origin_place, &candidate.place)
                    })
            }
            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
        })
        .collect()
}

fn previous_origin_loans(
    origins: &BTreeSet<ReferenceOriginId>,
    facts: &[ReferenceOrigin],
) -> Vec<ReferenceLoanId> {
    origins
        .iter()
        .filter_map(|origin| match &facts[origin.0] {
            ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                Some(loan.clone())
            }
            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => None,
        })
        .collect()
}

fn transfer_reference_facts(
    cfg: &mut ScalarReferenceCfg,
    deferred_policies: &BTreeMap<usize, CopyPolicy>,
    contracts: &[ConcreteReferenceOutputContract],
) -> Vec<ReferenceAnalysisError> {
    let original_points = cfg.points.clone();
    let grouped_installs = original_points
        .iter()
        .filter_map(|point| match &point.kind {
            CfgPointKind::CaptureAssignmentPrevious { writes } => Some(writes.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
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
                        if matches!(
                            &cfg.facts[origin.0],
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. }
                        ) {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason:
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            });
                        }
                    }
                    candidates.extend(assignment_pointer_candidates(place, &origins, &cfg.facts));
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
            if let CfgPointKind::CaptureAssignmentPrevious { writes } = &point.kind {
                for write in writes {
                    let origins = match &original_points[write.0].kind {
                        CfgPointKind::Assign { target, .. } => {
                            previous_place_origins(&state, &cfg.facts, target)
                        }
                        CfgPointKind::AssignThrough {
                            place,
                            pointer_point,
                            ..
                        } => {
                            let candidates = match &state.point_values[pointer_point.0] {
                                Some(origins) => {
                                    assignment_pointer_candidates(place, origins, &cfg.facts)
                                }
                                None => Vec::new(),
                            };
                            previous_candidate_origins(&state, &cfg.facts, &candidates)
                        }
                        _ => unreachable!("captured operation is an assignment install"),
                    };
                    state.assignment_previous.insert(*write, origins);
                }
            }
            let previous = match &point.kind {
                CfgPointKind::Assign { target, .. } => Some(if grouped_installs.contains(&id) {
                    state
                        .assignment_previous
                        .remove(&id)
                        .expect("grouped assignment previous origins")
                } else {
                    previous_place_origins(&state, &cfg.facts, target)
                }),
                CfgPointKind::AssignThrough { candidates, .. } => {
                    Some(if grouped_installs.contains(&id) {
                        state
                            .assignment_previous
                            .remove(&id)
                            .expect("grouped assignment previous origins")
                    } else {
                        previous_candidate_origins(&state, &cfg.facts, candidates)
                    })
                }
                _ => None,
            };
            if let Some(origins) = previous {
                match &mut point.kind {
                    CfgPointKind::Assign {
                        previous_origins,
                        previous_loans,
                        ..
                    }
                    | CfgPointKind::AssignThrough {
                        previous_origins,
                        previous_loans,
                        ..
                    } => {
                        *previous_loans = previous_origin_loans(&origins, &cfg.facts);
                        *previous_origins = origins;
                    }
                    _ => unreachable!("assignment previous origins"),
                }
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

struct CallResolutionModule<'a> {
    source: &'a SourceIdentity,
    items: &'a [ScalarItem],
    namespace_bindings: &'a [ScalarNamespaceBinding],
    structs: &'a [ScalarStruct],
}

impl<'a> CallResolutionModule<'a> {
    fn program(program: &'a ScalarProgram) -> Self {
        Self {
            source: &program.source,
            items: &program.items,
            namespace_bindings: &[],
            structs: &program.structs,
        }
    }

    fn module(module: &'a ScalarModule) -> Self {
        Self {
            source: &module.source,
            items: &module.items,
            namespace_bindings: &module.namespace_bindings,
            structs: &module.structs,
        }
    }
}

pub(super) fn resolve_program_call_sites(
    program: &ScalarProgram,
) -> Result<ModuleCallSites, ReferenceAnalysisError> {
    let modules = [CallResolutionModule::program(program)];
    resolve_module_call_sites(
        &program.source,
        &program.items,
        &[],
        &modules,
        program.resolved_assignment_outputs.as_map(),
    )
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ModuleCallSites {
    functions: BTreeMap<usize, FunctionCallSites>,
    pub(super) initialization: ModuleInitializationRegion,
}

#[cfg(test)]
impl ModuleCallSites {
    pub(super) fn function(&self, item: usize) -> &FunctionCallSites {
        &self.functions[&item]
    }

    pub(super) fn function_mut(&mut self, item: usize) -> &mut FunctionCallSites {
        self.functions
            .get_mut(&item)
            .expect("actual test function item ordinal")
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct OccurrenceOrdinal(pub(super) usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct BlockOrdinal(pub(super) usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum OccurrenceParent {
    Expression(OccurrenceOrdinal),
    Place(OccurrenceOrdinal),
    Block(BlockOrdinal),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum OccurrenceEdge {
    CallArgument(usize),
    BinaryLeft,
    BinaryRight,
    UnaryOperand,
    Condition,
    ThenBranch,
    ElseBranch,
    BlockBody,
    AggregateField(usize),
    AggregateElement(usize),
    ExpressionPlace,
    PlaceBase,
    PlaceIndex,
    DereferencePointer,
    BindingRoot { item: usize, output: usize },
    ExpressionItem { item: usize },
    FinalOutput { item: usize, output: usize },
    AssignmentRhs { item: usize, output: usize },
    AssignmentDestination { item: usize, destination: usize },
    WhileCondition { item: usize },
    WhileBody { item: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OccurrenceNode {
    Expression(OccurrenceOrdinal),
    Place(OccurrenceOrdinal),
    Block(BlockOrdinal),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OccurrenceStructure {
    pub(super) root: BlockOrdinal,
    pub(super) blocks: Vec<BlockRegion>,
    pub(super) edges: HashMap<(OccurrenceParent, OccurrenceEdge), OccurrenceNode>,
}

impl OccurrenceStructure {
    fn new(span: ByteSpan) -> Self {
        Self {
            root: BlockOrdinal(0),
            blocks: vec![BlockRegion::new(span, Vec::new())],
            edges: HashMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OccurrenceRole {
    Expression,
    Place,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OccurrenceMetadata {
    pub(super) role: OccurrenceRole,
    pub(super) span: ByteSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CallOccurrence {
    pub(super) ordinal: OccurrenceOrdinal,
    pub(super) site: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ParameterSlot {
    pub(super) ordinal: usize,
    pub(super) name: String,
    pub(super) span: ByteSpan,
    pub(super) ty: ScalarType,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DeclaredResultSlot {
    pub(super) ordinal: usize,
    pub(super) span: ByteSpan,
    pub(super) ty: ScalarType,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RootOutputType {
    pub(super) output: usize,
    pub(super) ty: ScalarType,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CallableBoundary {
    pub(super) parameters: Vec<ParameterSlot>,
    pub(super) results: Vec<DeclaredResultSlot>,
    pub(super) root_outputs: Vec<RootOutputType>,
    pub(super) generics: Vec<ScalarGenericParameter>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FunctionCallSites {
    pub(super) boundary: CallableBoundary,
    pub(super) sites: Vec<ResolvedCallSite>,
    pub(super) arms: Vec<FunctionCallSites>,
    pub(super) occurrences: Vec<OccurrenceMetadata>,
    pub(super) call_occurrences: Vec<CallOccurrence>,
    pub(super) structure: OccurrenceStructure,
    pub(super) assignments: HashMap<(BlockOrdinal, usize), AssignmentGroup>,
    pub(super) conditionals: HashMap<OccurrenceOrdinal, ConditionalRegion>,
    pub(super) loops: HashMap<(BlockOrdinal, usize), WhileRegion>,
    pub(super) short_circuits: HashMap<OccurrenceOrdinal, ShortCircuitRegion>,
}

pub(super) struct ProjectCallSiteResults {
    modules: Vec<Result<ModuleCallSites, ReferenceAnalysisError>>,
}

impl ProjectCallSiteResults {
    pub(super) fn initialization_region(
        &self,
        module_ordinal: usize,
    ) -> Option<&ModuleInitializationRegion> {
        let calls = self.modules.get(module_ordinal)?.as_ref().ok()?;
        Some(&calls.initialization)
    }
}

pub(super) fn resolve_project_call_sites(modules: &[ScalarModule]) -> ProjectCallSiteResults {
    let views = modules
        .iter()
        .map(CallResolutionModule::module)
        .collect::<Vec<_>>();
    ProjectCallSiteResults {
        modules: modules
            .iter()
            .map(|module| {
                resolve_module_call_sites(
                    &module.source,
                    &module.items,
                    &module.namespace_bindings,
                    &views,
                    module.resolved_assignment_outputs.as_map(),
                )
            })
            .collect(),
    }
}

struct ValidatedCallResolver<'a> {
    source: &'a SourceIdentity,
    function_span: ByteSpan,
    module_callables: &'a BTreeMap<(Option<String>, String), (SourceIdentity, ByteSpan)>,
    extern_callables: &'a BTreeMap<(String, String), (SourceIdentity, ByteSpan)>,
    modules: &'a [CallResolutionModule<'a>],
    assignment_outputs: &'a HashMap<ResolvedAssignmentOutputId, ScalarType>,
    sites: Vec<ResolvedCallSite>,
    occurrences: Vec<OccurrenceMetadata>,
    call_occurrences: Vec<CallOccurrence>,
    structure: OccurrenceStructure,
    assignments: HashMap<(BlockOrdinal, usize), AssignmentGroup>,
    conditionals: HashMap<OccurrenceOrdinal, ConditionalRegion>,
    loops: HashMap<(BlockOrdinal, usize), WhileRegion>,
    short_circuits: HashMap<OccurrenceOrdinal, ShortCircuitRegion>,
}

impl ValidatedCallResolver<'_> {
    fn record_occurrence(
        &mut self,
        role: OccurrenceRole,
        span: ByteSpan,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
    ) -> OccurrenceOrdinal {
        let ordinal = OccurrenceOrdinal(self.occurrences.len());
        self.occurrences.push(OccurrenceMetadata { role, span });
        let node = match role {
            OccurrenceRole::Expression => OccurrenceNode::Expression(ordinal),
            OccurrenceRole::Place => OccurrenceNode::Place(ordinal),
        };
        assert!(self.structure.edges.insert((parent, edge), node).is_none());
        ordinal
    }

    fn expression(
        &mut self,
        expression: &ScalarExpression,
        names: &BTreeMap<String, ReferenceBindingId>,
        types: &BTreeMap<String, ScalarType>,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
    ) -> Result<(), ReferenceAnalysisError> {
        self.expression_at(expression, names, types, parent, edge)
            .map(|_| ())
    }

    fn expression_at(
        &mut self,
        expression: &ScalarExpression,
        names: &BTreeMap<String, ReferenceBindingId>,
        types: &BTreeMap<String, ScalarType>,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
    ) -> Result<OccurrenceOrdinal, ReferenceAnalysisError> {
        let ordinal = self.record_occurrence(
            OccurrenceRole::Expression,
            expression_span(expression),
            parent,
            edge,
        );
        let parent = OccurrenceParent::Expression(ordinal);
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
                let site_index = if let Some((index, existing)) = self
                    .sites
                    .iter()
                    .enumerate()
                    .find(|(_, existing)| existing.source_span == site.source_span)
                {
                    if existing.target != site.target {
                        return Err(ReferenceAnalysisError::MissingContext {
                            source_span: site.source_span,
                            reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                        });
                    }
                    index
                } else {
                    let index = self.sites.len();
                    self.sites.push(site);
                    index
                };
                self.call_occurrences.push(CallOccurrence {
                    ordinal,
                    site: site_index,
                });
                for (position, argument) in arguments.iter().enumerate() {
                    self.expression(
                        argument,
                        names,
                        types,
                        parent,
                        OccurrenceEdge::CallArgument(position),
                    )?;
                }
            }
            ScalarExpression::Binary {
                operator,
                left,
                right,
                ..
            } => {
                let left =
                    self.expression_at(left, names, types, parent, OccurrenceEdge::BinaryLeft)?;
                let right =
                    self.expression_at(right, names, types, parent, OccurrenceEdge::BinaryRight)?;
                if matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
                    let evaluate_right_when = matches!(operator, BinaryOperator::And);
                    assert!(self
                        .short_circuits
                        .insert(
                            ordinal,
                            ShortCircuitRegion {
                                span: expression_span(expression),
                                left,
                                right,
                                evaluate_right_when,
                                bypass: !evaluate_right_when,
                                join: ordinal,
                            }
                        )
                        .is_none());
                }
            }
            ScalarExpression::Unary { operand, .. } => {
                self.expression(operand, names, types, parent, OccurrenceEdge::UnaryOperand)?
            }
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let condition =
                    self.expression_at(condition, names, types, parent, OccurrenceEdge::Condition)?;
                let then_block = self.block(
                    then_branch,
                    names,
                    types,
                    parent,
                    OccurrenceEdge::ThenBranch,
                )?;
                let else_block = self.block(
                    else_branch,
                    names,
                    types,
                    parent,
                    OccurrenceEdge::ElseBranch,
                )?;
                assert!(self
                    .conditionals
                    .insert(
                        ordinal,
                        ConditionalRegion::Value {
                            span: expression_span(expression),
                            condition,
                            then_block,
                            else_block,
                            resume: ordinal,
                        }
                    )
                    .is_none());
            }
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                let condition =
                    self.expression_at(condition, names, types, parent, OccurrenceEdge::Condition)?;
                let then_block = self.block(
                    then_branch,
                    names,
                    types,
                    parent,
                    OccurrenceEdge::ThenBranch,
                )?;
                assert!(self
                    .conditionals
                    .insert(
                        ordinal,
                        ConditionalRegion::Unit {
                            span: expression_span(expression),
                            condition,
                            then_block,
                            fallthrough: ordinal,
                        }
                    )
                    .is_none());
            }
            ScalarExpression::Block(block) => {
                self.block(block, names, types, parent, OccurrenceEdge::BlockBody)?;
            }
            ScalarExpression::RawAddress { place, .. }
            | ScalarExpression::CheckedAddress { place, .. }
            | ScalarExpression::Dereference { place, .. }
            | ScalarExpression::IndexedRead { place, .. } => {
                self.place(place, names, types, parent, OccurrenceEdge::ExpressionPlace)?
            }
            ScalarExpression::StructLiteral { fields, .. } => {
                for (position, field) in fields.iter().enumerate() {
                    self.expression(
                        &field.value,
                        names,
                        types,
                        parent,
                        OccurrenceEdge::AggregateField(position),
                    )?;
                }
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for (position, element) in elements.iter().enumerate() {
                    self.expression(
                        element,
                        names,
                        types,
                        parent,
                        OccurrenceEdge::AggregateElement(position),
                    )?;
                }
            }
            _ => {}
        }
        Ok(ordinal)
    }

    fn place(
        &mut self,
        place: &ScalarPlace,
        names: &BTreeMap<String, ReferenceBindingId>,
        types: &BTreeMap<String, ScalarType>,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
    ) -> Result<(), ReferenceAnalysisError> {
        self.place_at(place, names, types, parent, edge).map(|_| ())
    }

    fn place_at(
        &mut self,
        place: &ScalarPlace,
        names: &BTreeMap<String, ReferenceBindingId>,
        types: &BTreeMap<String, ScalarType>,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
    ) -> Result<OccurrenceOrdinal, ReferenceAnalysisError> {
        let ordinal = self.record_occurrence(
            OccurrenceRole::Place,
            scalar_place_target_span(place),
            parent,
            edge,
        );
        let parent = OccurrenceParent::Place(ordinal);
        match place {
            ScalarPlace::Name { .. } => {}
            ScalarPlace::Field { base, .. } => {
                self.place(base, names, types, parent, OccurrenceEdge::PlaceBase)?
            }
            ScalarPlace::Index { base, index, .. } => {
                self.place(base, names, types, parent, OccurrenceEdge::PlaceBase)?;
                self.expression(index, names, types, parent, OccurrenceEdge::PlaceIndex)?;
            }
            ScalarPlace::Dereference { pointer, .. } => self.expression(
                pointer,
                names,
                types,
                parent,
                OccurrenceEdge::DereferencePointer,
            )?,
        }
        Ok(ordinal)
    }

    fn block(
        &mut self,
        block: &ScalarBlock,
        outer: &BTreeMap<String, ReferenceBindingId>,
        outer_types: &BTreeMap<String, ScalarType>,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
    ) -> Result<BlockOrdinal, ReferenceAnalysisError> {
        let ordinal = BlockOrdinal(self.structure.blocks.len());
        self.structure
            .blocks
            .push(BlockRegion::new(block.span, Vec::new()));
        assert!(self
            .structure
            .edges
            .insert((parent, edge), OccurrenceNode::Block(ordinal))
            .is_none());
        self.block_items(block, outer, outer_types, ordinal)?;
        Ok(ordinal)
    }

    fn block_items(
        &mut self,
        block: &ScalarBlock,
        outer: &BTreeMap<String, ReferenceBindingId>,
        outer_types: &BTreeMap<String, ScalarType>,
        ordinal: BlockOrdinal,
    ) -> Result<(), ReferenceAnalysisError> {
        let parent = OccurrenceParent::Block(ordinal);
        let mut items = Vec::new();
        let mut names = outer.clone();
        let mut types = outer_types.clone();
        for (item_index, (item, role)) in block.items_with_output_roles().enumerate() {
            let kind = match item {
                ScalarBlockItem::LocalBinding(binding) => {
                    let ScalarBindingOutput::Evaluate(primary) = binding.output_source(0) else {
                        unreachable!("primary binding output evaluates its source")
                    };
                    let primary = self.expression_at(
                        primary,
                        &names,
                        &types,
                        parent,
                        OccurrenceEdge::BindingRoot {
                            item: item_index,
                            output: 0,
                        },
                    )?;
                    let output_count = match binding.output_origin {
                        ScalarBindingOutputOrigin::SingleExpression => binding.receivers.len(),
                        ScalarBindingOutputOrigin::IndependentExpressions => {
                            binding.output_values.len()
                        }
                    };
                    let mut secondary = Vec::with_capacity(output_count.saturating_sub(1));
                    for position in 1..output_count {
                        let output = match binding.output_source(position) {
                            ScalarBindingOutput::Evaluate(value) => {
                                let expression = self.expression_at(
                                    value,
                                    &names,
                                    &types,
                                    parent,
                                    OccurrenceEdge::BindingRoot {
                                        item: item_index,
                                        output: position,
                                    },
                                )?;
                                BindingOutput::Evaluate {
                                    position,
                                    expression,
                                }
                            }
                            ScalarBindingOutput::ProjectPrimary { output } => {
                                BindingOutput::ProjectPrimary { output }
                            }
                        };
                        secondary.push(output);
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
                    BlockItemKind::Binding(BindingRegion::new(primary, secondary, binding))
                }
                ScalarBlockItem::Expression(expression) => {
                    let edge = match &role {
                        ScalarBlockItemRole::Ordinary => {
                            OccurrenceEdge::ExpressionItem { item: item_index }
                        }
                        ScalarBlockItemRole::FinalOutput { output, .. } => {
                            OccurrenceEdge::FinalOutput {
                                item: item_index,
                                output: *output,
                            }
                        }
                    };
                    let expression =
                        self.expression_at(expression, &names, &types, parent, edge)?;
                    match role {
                        ScalarBlockItemRole::FinalOutput { output, metadata } => {
                            BlockItemKind::FinalOutput(FinalOutputSlot {
                                item: item_index,
                                output,
                                expression,
                                span: metadata.span,
                            })
                        }
                        ScalarBlockItemRole::Ordinary => BlockItemKind::Evaluation { expression },
                    }
                }
                ScalarBlockItem::Assignment(assignment) => {
                    let mut evaluations = Vec::new();
                    for (output, value) in assignment.rhs().evaluations() {
                        let expression = self.expression_at(
                            value,
                            &names,
                            &types,
                            parent,
                            OccurrenceEdge::AssignmentRhs {
                                item: item_index,
                                output,
                            },
                        )?;
                        evaluations.push(AssignmentEvaluation {
                            position: output,
                            expression,
                        });
                    }
                    let projections = assignment
                        .rhs()
                        .projected_outputs()
                        .map(|output| AssignmentProjection {
                            producer: evaluations[0],
                            output,
                        })
                        .collect();
                    let mut slots = Vec::new();
                    for (destination, target) in assignment.targets.iter().enumerate() {
                        let place = self.place_at(
                            &target.place,
                            &names,
                            &types,
                            parent,
                            OccurrenceEdge::AssignmentDestination {
                                item: item_index,
                                destination,
                            },
                        )?;
                        slots.push(AssignmentSlot {
                            position: destination,
                            destination: place,
                            target_span: target.target_span,
                            replacement: match self.assignment_outputs.get(
                                &ResolvedAssignmentOutputId {
                                    source: self.source.clone(),
                                    function_span: self.function_span,
                                    target_span: target.target_span,
                                    output_index: destination,
                                },
                            ) {
                                Some(ty) => AssignmentReplacement::Resolved(ty.clone()),
                                None => AssignmentReplacement::Missing,
                            },
                        });
                    }
                    assert!(self
                        .assignments
                        .insert(
                            (ordinal, item_index),
                            AssignmentGroup::new(
                                assignment.span,
                                evaluations,
                                projections,
                                slots,
                                BlockPosition {
                                    block: ordinal,
                                    item: item_index + 1
                                }
                            )
                        )
                        .is_none());
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
                    BlockItemKind::Assignment {
                        group: BlockPosition {
                            block: ordinal,
                            item: item_index,
                        },
                    }
                }
                ScalarBlockItem::While(while_expression) => {
                    let test = self.expression_at(
                        &while_expression.condition,
                        &names,
                        &types,
                        parent,
                        OccurrenceEdge::WhileCondition { item: item_index },
                    )?;
                    let body = self.block(
                        &while_expression.body,
                        &names,
                        &types,
                        parent,
                        OccurrenceEdge::WhileBody { item: item_index },
                    )?;
                    assert!(self
                        .loops
                        .insert(
                            (ordinal, item_index),
                            WhileRegion {
                                span: while_expression.span,
                                test,
                                body,
                                on_false: BlockPosition {
                                    block: ordinal,
                                    item: item_index + 1
                                },
                            }
                        )
                        .is_none());
                    BlockItemKind::While {
                        region: BlockPosition {
                            block: ordinal,
                            item: item_index,
                        },
                    }
                }
            };
            items.push(BlockItem {
                position: item_index,
                kind,
            });
        }
        self.structure.blocks[ordinal.0] = BlockRegion::new(block.span, items);
        Ok(())
    }
}

fn resolve_module_call_sites(
    source: &SourceIdentity,
    items: &[ScalarItem],
    namespace_bindings: &[ScalarNamespaceBinding],
    modules: &[CallResolutionModule<'_>],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
) -> Result<ModuleCallSites, ReferenceAnalysisError> {
    #[cfg(test)]
    stage_observations::record_call_sites();
    let current = modules.iter().find(|module| module.source == source);
    let mut module_callables = BTreeMap::new();
    let mut extern_callables = BTreeMap::new();
    for module in modules {
        let receiver = if module.source == source {
            None
        } else {
            current.and_then(|current| {
                current
                    .namespace_bindings
                    .iter()
                    .find(|binding| &binding.target == module.source)
                    .map(|binding| binding.binding.clone())
            })
        };
        if module.source != source && receiver.is_none() {
            continue;
        }
        for item in module.items {
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
                ScalarItem::Extern(extern_decl) if module.source == source => {
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
    let mut functions = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        if let ScalarItem::Function(function) = item {
            let calls = resolve_function_call_sites(
                source,
                function,
                &module_callables,
                &extern_callables,
                modules,
                assignment_outputs,
            )?;
            functions.insert(index, calls);
        }
    }
    Ok(ModuleCallSites {
        functions,
        initialization: module_initialization_region(source, items, namespace_bindings),
    })
}

#[cfg(test)]
pub(super) fn test_function_call_sites(
    source: &SourceIdentity,
    function: &ScalarFunction,
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
) -> FunctionCallSites {
    resolve_function_call_sites(
        source,
        function,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        assignment_outputs,
    )
    .expect("test callable without call declarations")
}

fn resolve_function_call_sites(
    source: &SourceIdentity,
    function: &ScalarFunction,
    module_callables: &BTreeMap<(Option<String>, String), (SourceIdentity, ByteSpan)>,
    extern_callables: &BTreeMap<(String, String), (SourceIdentity, ByteSpan)>,
    modules: &[CallResolutionModule<'_>],
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
) -> Result<FunctionCallSites, ReferenceAnalysisError> {
    let mut arms = Vec::new();
    for arm in &function.overload_arms {
        arms.push(resolve_function_call_sites(
            source,
            arm,
            module_callables,
            extern_callables,
            modules,
            assignment_outputs,
        )?);
    }
    let mut resolver = ValidatedCallResolver {
        source,
        function_span: function.span,
        module_callables,
        extern_callables,
        modules,
        assignment_outputs,
        sites: Vec::new(),
        occurrences: Vec::new(),
        call_occurrences: Vec::new(),
        structure: OccurrenceStructure::new(function.body.span),
        assignments: HashMap::new(),
        conditionals: HashMap::new(),
        loops: HashMap::new(),
        short_circuits: HashMap::new(),
    };
    let mut parameter_names = BTreeMap::new();
    let mut parameter_types = BTreeMap::new();
    let ScalarType::Callable {
        parameters,
        outputs,
    } = &function.signature
    else {
        unreachable!("validated function signature")
    };
    let boundary = CallableBoundary {
        parameters: function
            .parameters
            .iter()
            .zip(&function.parameter_spans)
            .enumerate()
            .map(|(ordinal, (name, span))| ParameterSlot {
                ordinal,
                name: name.clone(),
                span: *span,
                ty: parameters[ordinal].clone(),
            })
            .collect(),
        results: outputs
            .outputs
            .iter()
            .enumerate()
            .map(|(ordinal, output)| DeclaredResultSlot {
                ordinal,
                span: output.span,
                ty: output.ty.clone(),
            })
            .collect(),
        root_outputs: function
            .body
            .final_output_values
            .iter()
            .enumerate()
            .map(|(output, metadata)| RootOutputType {
                output,
                ty: metadata.ty.clone(),
            })
            .collect(),
        generics: function.generic_parameters.clone(),
    };
    for slot in &boundary.parameters {
        parameter_types.insert(slot.name.clone(), slot.ty.clone());
        parameter_names.insert(
            slot.name.clone(),
            ReferenceBindingId {
                source: source.clone(),
                function_span: function.span,
                declaration_span: slot.span,
                block_span: function.body.span,
                kind: ReferenceBindingKind::Declared,
            },
        );
    }
    resolver.block_items(
        &function.body,
        &parameter_names,
        &parameter_types,
        resolver.structure.root,
    )?;
    Ok(FunctionCallSites {
        boundary,
        sites: resolver.sites,
        arms,
        occurrences: resolver.occurrences,
        call_occurrences: resolver.call_occurrences,
        structure: resolver.structure,
        assignments: resolver.assignments,
        conditionals: resolver.conditionals,
        loops: resolver.loops,
        short_circuits: resolver.short_circuits,
    })
}

#[cfg(test)]
pub(super) fn record_module_reference_origins(
    source: &SourceIdentity,
    items: &mut [ScalarItem],
    call_sites: &ModuleCallSites,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
) -> Vec<ReferenceAnalysisError> {
    let (updates, errors) =
        derive_module_reference_origins(source, items, call_sites, structs, enums);
    install_module_reference_results(items, updates);
    errors
}

fn derive_module_reference_origins(
    source: &SourceIdentity,
    items: &[ScalarItem],
    call_sites: &ModuleCallSites,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
) -> (
    Vec<(usize, FunctionReferenceResult)>,
    Vec<ReferenceAnalysisError>,
) {
    #[cfg(test)]
    stage_observations::record_reference_origins();
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
    let mut updates = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if let ScalarItem::Function(function) = item {
            updates.push((
                index,
                derive_function_reference_result(
                    source,
                    function,
                    &module_names,
                    &call_sites.functions[&index],
                    structs,
                    enums,
                    &mut errors,
                ),
            ));
        }
    }
    (updates, errors)
}

fn install_module_reference_results(
    items: &mut [ScalarItem],
    updates: Vec<(usize, FunctionReferenceResult)>,
) {
    for (index, result) in updates {
        let ScalarItem::Function(function) = &mut items[index] else {
            unreachable!("function result ordinal")
        };
        result.install(function);
    }
}

#[cfg(test)]
pub(super) fn record_project_reference_origins(
    modules: &mut [ScalarModule],
    call_sites: &ProjectCallSiteResults,
) -> Vec<ReferenceAnalysisError> {
    analyze_project_references(modules, call_sites).install(modules)
}

pub(super) struct ProjectReferenceResults {
    updates: Vec<(usize, Vec<(usize, FunctionReferenceResult)>)>,
    errors: Vec<ReferenceAnalysisError>,
}

impl ProjectReferenceResults {
    pub(super) fn errors(&self) -> &[ReferenceAnalysisError] {
        &self.errors
    }

    pub(super) fn reference_cfg(
        &self,
        module_ordinal: usize,
        item_ordinal: usize,
        arm_index: Option<usize>,
    ) -> Option<&ScalarReferenceCfg> {
        let module = self
            .updates
            .binary_search_by_key(&module_ordinal, |(ordinal, _)| *ordinal)
            .ok()?;
        let items = &self.updates[module].1;
        let item = items
            .binary_search_by_key(&item_ordinal, |(ordinal, _)| *ordinal)
            .ok()?;
        let result = &items[item].1;
        let body = match arm_index {
            Some(arm) => result.arms.get(arm)?,
            None => result,
        };
        Some(&body.cfg)
    }

    pub(super) fn install(self, modules: &mut [ScalarModule]) -> Vec<ReferenceAnalysisError> {
        for (index, results) in self.updates {
            install_module_reference_results(&mut modules[index].items, results);
        }
        self.errors
    }
}

pub(super) fn analyze_project_references(
    modules: &[ScalarModule],
    call_sites: &ProjectCallSiteResults,
) -> ProjectReferenceResults {
    let (mut updates, mut errors, inventory) =
        derive_and_converge_project_references(modules, call_sites);
    if errors.is_empty() && !modules.is_empty() {
        let (structs, enums) =
            inventory.expect("successful module call lookup built aggregate inventory");
        let mut views = updates
            .iter_mut()
            .map(|(index, results)| ModuleReferenceView {
                source: &modules[*index].source,
                items: &modules[*index].items,
                results,
            })
            .collect::<Vec<_>>();
        errors.extend(specialize_reference_results(&mut views, &structs, &enums));
    }
    ProjectReferenceResults { updates, errors }
}

fn derive_and_converge_project_references(
    modules: &[ScalarModule],
    call_sites: &ProjectCallSiteResults,
) -> (
    Vec<(usize, Vec<(usize, FunctionReferenceResult)>)>,
    Vec<ReferenceAnalysisError>,
    Option<(Vec<ScalarStruct>, Vec<ScalarEnum>)>,
) {
    let (mut updates, mut errors, inventory) =
        derive_project_reference_origins(modules, call_sites);
    if errors.is_empty() {
        let mut views = updates
            .iter_mut()
            .map(|(index, results)| ModuleReferenceView {
                source: &modules[*index].source,
                items: &modules[*index].items,
                results,
            })
            .collect::<Vec<_>>();
        errors.extend(resolve_reference_output_contracts(&mut views));
    }
    (updates, errors, inventory)
}

fn derive_project_reference_origins(
    modules: &[ScalarModule],
    call_sites: &ProjectCallSiteResults,
) -> (
    Vec<(usize, Vec<(usize, FunctionReferenceResult)>)>,
    Vec<ReferenceAnalysisError>,
    Option<(Vec<ScalarStruct>, Vec<ScalarEnum>)>,
) {
    let mut inventory = None;
    let mut updates = Vec::new();
    let mut errors = Vec::new();
    for (index, (module, calls)) in modules.iter().zip(&call_sites.modules).enumerate() {
        match calls {
            Ok(sites) => {
                let (structs, enums) = inventory.get_or_insert_with(|| {
                    (
                        modules
                            .iter()
                            .flat_map(|module| module.structs.iter().cloned())
                            .collect::<Vec<_>>(),
                        modules
                            .iter()
                            .flat_map(|module| module.enums.iter().cloned())
                            .collect::<Vec<_>>(),
                    )
                });
                let (results, current_errors) = derive_module_reference_origins(
                    &module.source,
                    &module.items,
                    sites,
                    structs,
                    enums,
                );
                updates.push((index, results));
                errors.extend(current_errors);
            }
            Err(error) => errors.push(error.clone()),
        }
    }
    (updates, errors, inventory)
}

struct ModuleReferenceView<'a> {
    source: &'a SourceIdentity,
    items: &'a [ScalarItem],
    results: &'a mut [(usize, FunctionReferenceResult)],
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
    result: &FunctionReferenceResult,
    templates: &[ReferenceOutputContract],
    concrete: &mut Vec<ConcreteReferenceOutputContract>,
) {
    for arm in &result.arms {
        collect_concrete_output_contracts(arm, templates, concrete);
    }
    for point in &result.cfg.initial_points {
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
    result: &FunctionReferenceResult,
    declaration_span: ByteSpan,
    body: ReferenceCallableBody,
    contracts: &mut Vec<ReferenceOutputContract>,
) {
    for point in &result.cfg.points {
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
                .map(|id| result.cfg.facts[id.0].clone())
                .collect(),
        });
    }
    for (index, arm) in function.overload_arms.iter().enumerate() {
        collect_function_output_contracts(
            source,
            arm,
            &result.arms[index],
            declaration_span,
            ReferenceCallableBody::Overload(index),
            contracts,
        );
    }
}

fn transfer_function_output_contracts(
    result: &mut FunctionReferenceResult,
    contracts: &[ConcreteReferenceOutputContract],
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    for arm in &mut result.arms {
        errors.extend(transfer_function_output_contracts(arm, contracts));
    }
    if !result.cfg.initial_points.is_empty() {
        result.cfg.points = result.cfg.initial_points.clone();
        errors.extend(transfer_reference_facts(
            &mut result.cfg,
            &BTreeMap::new(),
            contracts,
        ));
    }
    errors
}

fn resolve_reference_output_contracts(
    modules: &mut [ModuleReferenceView<'_>],
) -> Vec<ReferenceAnalysisError> {
    #[cfg(test)]
    stage_observations::record_output_contracts();
    let mut contracts = Vec::new();
    loop {
        let mut concrete = Vec::new();
        for module in modules.iter() {
            for (_, result) in module.results.iter() {
                collect_concrete_output_contracts(result, &contracts, &mut concrete);
            }
        }
        let mut errors = Vec::new();
        for module in modules.iter_mut() {
            for (_, result) in module.results.iter_mut() {
                errors.extend(transfer_function_output_contracts(result, &concrete));
            }
        }
        let mut next = Vec::new();
        for module in modules.iter() {
            for (index, result) in module.results.iter() {
                let ScalarItem::Function(function) = &module.items[*index] else {
                    unreachable!("function result ordinal")
                };
                collect_function_output_contracts(
                    module.source,
                    function,
                    result,
                    function.name_span,
                    ReferenceCallableBody::Declaration,
                    &mut next,
                );
            }
        }
        if next == contracts {
            return errors;
        }
        contracts = next;
    }
}

struct ReferenceTemplateIndex<'a> {
    modules: HashMap<
        &'a SourceIdentity,
        HashMap<ByteSpan, (&'a ScalarFunction, &'a FunctionReferenceResult)>,
    >,
}

impl<'a> ReferenceTemplateIndex<'a> {
    fn new(modules: &'a [ModuleReferenceView<'_>]) -> Self {
        let mut index = HashMap::new();
        for module in modules {
            index.entry(module.source).or_insert_with(|| {
                let mut declarations = HashMap::new();
                for (ordinal, result) in module.results.iter() {
                    let ScalarItem::Function(function) = &module.items[*ordinal] else {
                        unreachable!("function result ordinal")
                    };
                    declarations
                        .entry(function.name_span)
                        .or_insert((function, result));
                }
                declarations
            });
        }
        Self { modules: index }
    }
}

struct FunctionPolicyResult {
    policies: Vec<SpecializedCopyPolicy>,
    arms: Vec<FunctionPolicyResult>,
}

impl FunctionPolicyResult {
    fn apply(self, result: &mut FunctionReferenceResult) {
        for (update, arm) in self.arms.into_iter().zip(&mut result.arms) {
            update.apply(arm);
        }
        result.cfg.specialized_policies.extend(self.policies);
    }
}

fn specialize_reference_results(
    modules: &mut [ModuleReferenceView<'_>],
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
) -> Vec<ReferenceAnalysisError> {
    let templates = ReferenceTemplateIndex::new(modules);
    let mut errors = Vec::new();
    let mut updates = Vec::new();
    for module in modules.iter() {
        #[cfg(test)]
        stage_observations::record_copy_specialization();
        let mut module_updates = Vec::new();
        for (ordinal, result) in module.results.iter() {
            let ScalarItem::Function(function) = &module.items[*ordinal] else {
                unreachable!("function result ordinal")
            };
            let (update, current_errors) = derive_function_specialized_copy_policies(
                function, result, &templates, structs, enums,
            );
            module_updates.push(update);
            errors.extend(current_errors);
        }
        updates.push(module_updates);
    }
    drop(templates);
    for (module, updates) in modules.iter_mut().zip(updates) {
        for ((_, result), update) in module.results.iter_mut().zip(updates) {
            update.apply(result);
        }
    }
    errors
}

fn derive_function_specialized_copy_policies(
    function: &ScalarFunction,
    result: &FunctionReferenceResult,
    templates: &ReferenceTemplateIndex<'_>,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
) -> (FunctionPolicyResult, Vec<ReferenceAnalysisError>) {
    let mut errors = Vec::new();
    let mut arms = Vec::new();
    let mut policies = Vec::new();
    for (arm, result) in function.overload_arms.iter().zip(&result.arms) {
        let (update, current_errors) =
            derive_function_specialized_copy_policies(arm, result, templates, structs, enums);
        arms.push(update);
        errors.extend(current_errors);
    }
    let generic_parameters = function
        .generic_parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect::<BTreeSet<_>>();
    for point in &result.cfg.points {
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
        let Some((declaration, declaration_result)) = templates
            .modules
            .get(source)
            .and_then(|module| module.get(declaration_span))
        else {
            errors.push(error());
            continue;
        };
        let template = match concrete {
            ConcreteCallSelection::Overload(selection) => declaration
                .overload_arms
                .get(selection.arm_index)
                .zip(declaration_result.arms.get(selection.arm_index)),
            ConcreteCallSelection::Declaration => Some((*declaration, *declaration_result)),
        };
        let Some((template, template_result)) = template else {
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
        for template_point in &template_result.cfg.points {
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
                    policies.push(SpecializedCopyPolicy {
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
            let mut concrete_cfg = template_result.cfg.clone();
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
    (FunctionPolicyResult { policies, arms }, errors)
}

struct FunctionReferenceResult {
    cfg: ScalarReferenceCfg,
    arms: Vec<FunctionReferenceResult>,
}

impl FunctionReferenceResult {
    fn install(self, function: &mut ScalarFunction) {
        assert_eq!(self.arms.len(), function.overload_arms.len());
        for (result, arm) in self.arms.into_iter().zip(&mut function.overload_arms) {
            result.install(arm);
        }
        function.reference_cfg = self.cfg;
    }
}

#[cfg(test)]
pub(super) fn record_function_reference_origins(
    source: &SourceIdentity,
    function: &mut ScalarFunction,
    module_names: &HashSet<String>,
    call_sites: &FunctionCallSites,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
) -> Vec<ReferenceAnalysisError> {
    let mut errors = Vec::new();
    let result = derive_function_reference_result(
        source,
        function,
        module_names,
        call_sites,
        structs,
        enums,
        &mut errors,
    );
    result.install(function);
    errors
}

fn derive_function_reference_result(
    source: &SourceIdentity,
    function: &ScalarFunction,
    module_names: &HashSet<String>,
    call_sites: &FunctionCallSites,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    errors: &mut Vec<ReferenceAnalysisError>,
) -> FunctionReferenceResult {
    let entry_errors = errors.len();
    let mut arms = Vec::new();
    assert_eq!(function.overload_arms.len(), call_sites.arms.len());
    for (arm, calls) in function.overload_arms.iter().zip(&call_sites.arms) {
        arms.push(derive_function_reference_result(
            source,
            arm,
            module_names,
            calls,
            structs,
            enums,
            errors,
        ));
    }
    let (cfg, current_errors) = derive_function_reference_cfg(
        source,
        function,
        module_names,
        call_sites,
        structs,
        enums,
        errors.len() == entry_errors,
    );
    errors.extend(current_errors);
    FunctionReferenceResult { cfg, arms }
}

fn derive_function_reference_cfg(
    source: &SourceIdentity,
    function: &ScalarFunction,
    module_names: &HashSet<String>,
    call_sites: &FunctionCallSites,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    prior_children_succeeded: bool,
) -> (ScalarReferenceCfg, Vec<ReferenceAnalysisError>) {
    let mut errors = Vec::new();
    let mut builder =
        ReferenceFlowBuilder::new(source, function, module_names, call_sites, structs, enums);
    builder.point(
        CfgPointKind::Entry,
        function.span,
        ReferenceScopeId(0),
        BTreeSet::new(),
    );
    for slot in &call_sites.boundary.parameters {
        builder.names.insert(
            slot.name.clone(),
            ReferenceBindingId {
                source: source.clone(),
                function_span: function.span,
                declaration_span: slot.span,
                block_span: function.body.span,
                kind: ReferenceBindingKind::Declared,
            },
        );
    }
    for slot in &call_sites.boundary.parameters {
        let name = &slot.name;
        let span = &slot.span;
        let ty = &slot.ty;
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
                parameter: slot.ordinal,
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
    match builder.block(&function.body, call_sites.structure.root) {
        Err(error) => errors.push(error),
        Ok(result) => {
            if matches!(
                call_sites.boundary.results.first(),
                Some(DeclaredResultSlot {
                    ty: ScalarType::CheckedReference { .. },
                    ..
                })
            ) && result.is_none()
                && call_sites.structure.blocks[call_sites.structure.root.0]
                    .final_outputs()
                    .next()
                    .is_some()
            {
                let span = call_sites.structure.blocks[call_sites.structure.root.0]
                    .final_outputs()
                    .next()
                    .expect("root final output")
                    .span;
                errors.push(builder.missing_context(
                    span,
                    ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                ));
            }
        }
    }
    if prior_children_succeeded && errors.is_empty() && !builder.cfg.points.is_empty() {
        builder.cfg.initial_points = builder.cfg.points.clone();
        let _ = transfer_reference_facts(&mut builder.cfg, &BTreeMap::new(), &[]);
    }
    (builder.cfg, errors)
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

#[cfg(test)]
use super::control::NamespaceTarget;
use super::control::{
    module_initialization_region, BindingAllocationLengthSlot, BindingInitialization,
    BindingOutput, BindingOutputs, BindingReceiverSlot, BindingRegion, BlockItem, BlockItemKind,
    BlockPosition, BlockRegion, CheckedCallSignature, CheckedOccurrenceTypes,
    ConditionalOutputProjection, ConditionalRegion, FinalOutputSlot, FoldedOccurrence,
    InitializationDestination, InitializationIdentities, InitializationLocal, InitializationMember,
    InitializationName, InitializationStorage, ModuleInitializationCallTarget,
    ModuleInitializationDetail, ModuleInitializationRegion, ModuleInitializationStepKind,
    ModuleInitializationTopology, ShortCircuitRegion, SyntheticOccurrence,
};
use super::control::{
    AssignmentEvaluation, AssignmentGroup, AssignmentProjection, AssignmentReplacement,
    AssignmentSlot, BindingDefinition, CallablePointFacts, FrozenInstall, ScalarDescriptorCandidates, ScalarDescriptorInstall,
};
use super::inference::CheckedMember;
use super::*;

pub(crate) mod aggregate;
pub(super) mod self_tail;

#[cfg(test)]
mod input_tests;

#[cfg(test)]
pub(super) mod call_site_comparisons {
    use std::cell::Cell;

    thread_local! {
        static ACTIVE: Cell<Option<usize>> = const { Cell::new(None) };
    }

    pub(in crate::scalar) struct Scope(Option<usize>);

    impl Scope {
        pub(in crate::scalar) fn new() -> Self {
            Self(ACTIVE.replace(Some(0)))
        }

        pub(in crate::scalar) fn count(&self) -> usize {
            ACTIVE.get().expect("active call-site comparisons")
        }
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            ACTIVE.set(self.0);
        }
    }

    pub(super) fn record() {
        if let Some(count) = ACTIVE.get() {
            ACTIVE.set(Some(count + 1));
        }
    }
}

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
    pub(super) fn install_retaining_aggregate(self, program: &mut ScalarProgram) -> (Vec<ReferenceAnalysisError>, aggregate::InstalledAggregateFacts) {
        let mut facts = aggregate::InstalledAggregateFacts::default();
        for (ordinal, result) in self.updates {
            let ScalarItem::Function(function) = &mut program.items[ordinal] else {
                unreachable!("function result ordinal");
            };
            result.install_body(function, ordinal, None, Some(&mut facts));
        }
        (self.errors, facts)
    }

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
        false,
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

#[derive(Eq, Ord, PartialEq, PartialOrd)]
enum ReferenceOriginKey {
    Fresh((u32, u32), (u32, u32)),
    BorrowedFrom(usize, (u32, u32)),
    Null((u32, u32)),
    Invalid((u32, u32), (u32, u32)),
}

impl ReferenceOriginKey {
    fn of(fact: &ReferenceOrigin) -> Self {
        let range = |span: ByteSpan| (span.start, span.end);
        match fact {
            ReferenceOrigin::Fresh { allocation, loan } => {
                Self::Fresh(range(loan.creation_span), range(allocation.creation_span))
            }
            ReferenceOrigin::BorrowedFrom { parameter, loan } => {
                Self::BorrowedFrom(*parameter, range(loan.creation_span))
            }
            ReferenceOrigin::Null { span } => Self::Null(range(*span)),
            ReferenceOrigin::Invalid { origin_span, conflict_span } => {
                Self::Invalid(range(*origin_span), range(*conflict_span))
            }
        }
    }
}

enum ReferenceOriginBucket {
    Single(ReferenceOriginId),
    Shared(Vec<ReferenceOriginId>),
}

#[derive(Default)]
struct ReferenceOriginIndex {
    by_key: BTreeMap<ReferenceOriginKey, ReferenceOriginBucket>,
}

impl ReferenceOriginIndex {
    fn intern(
        &mut self,
        facts: &mut Vec<ReferenceOrigin>,
        origin: ReferenceOrigin,
    ) -> ReferenceOriginId {
        use std::collections::btree_map::Entry;

        let id = ReferenceOriginId(facts.len());
        match self.by_key.entry(ReferenceOriginKey::of(&origin)) {
            Entry::Occupied(mut entry) => {
                let bucket = entry.get_mut();
                match bucket {
                    ReferenceOriginBucket::Single(first) => {
                        if facts[first.0] == origin {
                            return *first;
                        }
                        *bucket = ReferenceOriginBucket::Shared(vec![*first, id]);
                    }
                    ReferenceOriginBucket::Shared(ids) => {
                        if let Some(previous) = ids.iter().copied().find(|id| facts[id.0] == origin) {
                            return previous;
                        }
                        ids.push(id);
                    }
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(ReferenceOriginBucket::Single(id));
            }
        }
        facts.push(origin);
        id
    }
}

pub(super) struct ReferenceFlowBuilder<'a> {
    capture_call_points: bool,
    aggregate_facts: aggregate::AggregateFlowFacts,
    aggregate_result: aggregate::AggregateFlowResult,
    returning_leaf_prefix: bool,
    formal_forward_shape: Option<(Option<CfgPointId>, Option<CfgPointId>)>,
    independent_checked_query: bool,
    checked_scalar_storage: BTreeSet<ReferenceBindingId>,
    checked_result_origins: BTreeSet<ReferenceOriginId>,
    proof_points: CallablePointFacts,
    deferred_reborrow_roots: Vec<(ReferenceBindingId, super::control::DeferredReborrowRoot, CfgPointId, ReferenceOriginId)>,
    deferred_local_reborrow: bool,
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
    origin_index: ReferenceOriginIndex,
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

#[derive(Default)]
struct ReferenceBlockResult {
    scalar: Option<ReferenceValue>,
    output_points: Vec<CfgPointId>,
}

struct ReferenceConditionalArm {
    end: CfgPointId,
    result: ReferenceBlockResult,
}

enum ReferenceAssignmentDestination<'a> {
    RawName,
    PreparedName(&'a DirectDestinationIdentity),
    EvaluationOnly,
    Through(CfgPointId),
    Resolved(ReferencePlaceId),
}

#[derive(Clone, Copy)]
enum CheckedReadSource<'a> {
    Place(OccurrenceOrdinal),
    SyntheticMember,
    PreparedMember {
        binding: &'a ReferenceBindingId,
        ty: &'a ScalarType,
    },
}

struct ReferencePlaceRoot<'a> {
    place: &'a ScalarPlace,
    ordinal: OccurrenceOrdinal,
}

enum PreparedPlaceRoot<'a> {
    Lexical(&'a ReferenceBindingId),
    NonLexical,
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
            capture_call_points: false,
            aggregate_facts: aggregate::AggregateFlowFacts::default(),
            aggregate_result: aggregate::AggregateFlowResult::default(),
            returning_leaf_prefix: true,
            formal_forward_shape: None,
            independent_checked_query: aggregate::checked_scalar_output(&call_sites.boundary.root_outputs),
            checked_scalar_storage: BTreeSet::new(),
            checked_result_origins: BTreeSet::new(),
            proof_points: CallablePointFacts::default(),
            deferred_reborrow_roots: Vec::new(),
            deferred_local_reborrow: deferred_named_reborrow_allowed(function, call_sites)
                && call_sites.call_occurrences.is_empty()
                && call_sites.boundary.parameters.iter().all(|slot| aggregate::ordinary_value(&slot.ty)),
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
            origin_index: ReferenceOriginIndex::default(),
            cfg: ScalarReferenceCfg {
                primary_call_points: HashMap::new(),
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
                AssignmentReplacement::StructuralOnly => {
                    unreachable!("reference assignments are owned by a callable")
                }
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
        self.origin_index.intern(&mut self.cfg.facts, origin)
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
        self.returning_leaf_prefix = self.returning_leaf_prefix && aggregate::returning_leaf_point(&kind);
        let id = CfgPointId(self.cfg.points.len());
        self.formal_forward_shape = self.formal_forward_shape.and_then(|(moved, call)| {
            match &kind {
                CfgPointKind::Move { place, .. }
                    if moved.is_none() && call.is_none() && place.projections.is_empty() =>
                    Some((Some(id), None)),
                CfgPointKind::Call { output: 0, target: ResolvedCallTarget::ModuleCallable { .. },
                    argument_points, .. }
                    if moved.is_some() && call.is_none() && argument_points.len() == 1 =>
                    Some((moved, Some(id))),
                CfgPointKind::ReturnOutput { output: 0, value_point, checked: true }
                    if self.independent_checked_query && call == Some(*value_point) =>
                    Some((moved, call)),
                _ if aggregate::returning_leaf_point(&kind) => Some((moved, call)),
                _ => None,
            }
        });
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
            .ok_or_else(|| self.missing_binding(name, span))
    }

    fn missing_binding(&self, name: &str, span: ByteSpan) -> ReferenceAnalysisError {
        ReferenceAnalysisError::MissingBinding {
            source_span: SourceSpan::new(self.source.clone(), span),
            name: name.to_owned(),
        }
    }

    fn named_expression_binding(
        &self,
        name: &str,
        span: ByteSpan,
        ordinal: OccurrenceOrdinal,
    ) -> Result<ReferenceBindingId, ReferenceAnalysisError> {
        match self.prepared_name_identity(ordinal, span)? {
            Some(PlainNameIdentity::Lexical(binding)) => Ok(binding.clone()),
            Some(PlainNameIdentity::ModuleValue(_) | PlainNameIdentity::Null) => {
                Err(self.missing_binding(name, span))
            }
            None => self.binding(name, span),
        }
    }

    fn named_place_binding(
        &self,
        name: &str,
        span: ByteSpan,
        ordinal: OccurrenceOrdinal,
    ) -> Result<ReferenceBindingId, ReferenceAnalysisError> {
        match self.prepared_place_root(ordinal, span)? {
            Some(PreparedPlaceRoot::Lexical(binding)) => Ok(binding.clone()),
            Some(PreparedPlaceRoot::NonLexical) => Err(self.missing_binding(name, span)),
            None => self.binding(name, span),
        }
    }

    fn prepared_name_identity(
        &self,
        ordinal: OccurrenceOrdinal,
        span: ByteSpan,
    ) -> Result<Option<&PlainNameIdentity>, ReferenceAnalysisError> {
        match &self.call_sites.plain_names {
            PlainNameFacts::Raw => Ok(None),
            PlainNameFacts::Prepared(identities) => {
                identities.get(&ordinal).map(Some).ok_or_else(|| {
                    self.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
                })
            }
        }
    }

    fn prepared_member_identity(
        &self,
        ordinal: OccurrenceOrdinal,
        span: ByteSpan,
    ) -> Result<Option<&'a PreparedMemberIdentity>, ReferenceAnalysisError> {
        let MemberFacts::Prepared(members) = &self.call_sites.members else {
            return Ok(None);
        };
        members.get(&ordinal).map(Some).ok_or_else(|| {
            self.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
        })
    }

    fn checked_member_type(
        &self,
        ordinal: OccurrenceOrdinal,
        span: ByteSpan,
    ) -> Result<&'a ScalarType, ReferenceAnalysisError> {
        self.call_sites
            .structure
            .checked_types
            .expressions
            .get(&ordinal)
            .ok_or_else(|| {
                self.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
            })
    }

    fn raw_member_field(
        &self,
        id: &ScalarStructId,
        name: &str,
        span: ByteSpan,
    ) -> Result<&'a ScalarStructField, ReferenceAnalysisError> {
        self.structs
            .iter()
            .find(|structure| structure.id == *id)
            .and_then(|structure| structure.fields.iter().find(|field| field.name == name))
            .ok_or_else(|| {
                self.missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
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
        Ok(self.reference_place(place, binding, parent))
    }

    fn reference_place(
        &self,
        place: &ScalarPlace,
        binding: ReferenceBindingId,
        parent: Option<&ReferenceLoanId>,
    ) -> ReferencePlaceId {
        let mut projections = Vec::new();
        collect_reference_projections(place, parent, &mut projections);
        ReferencePlaceId {
            source: self.source.clone(),
            function_span: self.function_span,
            declaration_span: binding.declaration_span,
            block_span: binding.block_span,
            binding,
            projections,
            place: place.clone(),
        }
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
        ordinal: OccurrenceOrdinal,
        mutability: ScalarReferenceMutability,
        span: ByteSpan,
    ) -> Result<BTreeSet<ReferenceOriginId>, ReferenceAnalysisError> {
        let is_null = match raw {
            ScalarExpression::Name { name, span } => {
                match self.prepared_name_identity(ordinal, *span)? {
                    Some(identity) => matches!(identity, PlainNameIdentity::Null),
                    None => name == "null",
                }
            }
            _ => false,
        };
        let fact = if is_null {
            ReferenceOrigin::Null { span }
        } else {
            let binding = match raw {
                ScalarExpression::Name {
                    name,
                    span: raw_span,
                } => Some(self.named_expression_binding(name, *raw_span, ordinal)?),
                ScalarExpression::RawAddress { place, .. } => {
                    let root = self.place_root(
                        place,
                        self.place_ordinal(
                            OccurrenceParent::Expression(ordinal),
                            OccurrenceEdge::ExpressionPlace,
                        ),
                    );
                    match root.place {
                        ScalarPlace::Name { name, span } => {
                            Some(self.named_place_binding(name, *span, root.ordinal)?)
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let binding = binding.unwrap_or_else(|| ReferenceBindingId {
                source: self.source.clone(),
                function_span: self.function_span,
                declaration_span: span,
                block_span: self.function_span,
                kind: ReferenceBindingKind::CallOutput {
                    point: CfgPointId(self.cfg.points.len()),
                    output: 0,
                },
            });
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

    fn conditional_branches(
        &mut self,
        condition: &ScalarExpression,
        then_branch: &ScalarBlock,
        else_branch: Option<&ScalarBlock>,
        scope: ReferenceScopeId,
        region: &ConditionalRegion,
        projected_origins: bool,
    ) -> Result<[ReferenceConditionalArm; 2], ReferenceAnalysisError> {
        let (condition_ordinal, then_block, else_block) = match region {
            ConditionalRegion::Value {
                condition,
                then_block,
                else_block,
                ..
            } => (*condition, *then_block, Some(*else_block)),
            ConditionalRegion::Unit {
                condition,
                then_block,
                ..
            } => (*condition, *then_block, None),
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
        let then_arm = ReferenceConditionalArm {
            result: self.block_result(then_branch, then_block, projected_origins)?,
            end: self.cursor.expect("then branch terminal"),
        };
        let then_checked = self.checked.clone();

        self.names = incoming_names;
        self.origins = incoming_origins.clone();
        self.checked = incoming_checked.clone();
        self.cursor = Some(branch);
        let else_arm = match (else_branch, else_block) {
            (Some(else_branch), Some(else_block)) => ReferenceConditionalArm {
                result: self.block_result(else_branch, else_block, projected_origins)?,
                end: self.cursor.expect("else branch terminal"),
            },
            (None, None) => ReferenceConditionalArm {
                result: ReferenceBlockResult::default(),
                end: branch,
            },
            _ => unreachable!("conditional syntax and region disagree on else arm"),
        };
        self.checked.extend(then_checked);
        self.origins.clear();
        self.cursor = None;
        Ok([then_arm, else_arm])
    }

    fn conditional(
        &mut self,
        condition: &ScalarExpression,
        then_branch: &ScalarBlock,
        else_branch: Option<&ScalarBlock>,
        span: ByteSpan,
        scope: ReferenceScopeId,
        region: &ConditionalRegion,
        projected_origins: bool,
        checked_result: bool,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let [then_arm, else_arm] = self.conditional_branches(
            condition,
            then_branch,
            else_branch,
            scope,
            region,
            projected_origins,
        )?;
        let joined_value = then_arm.result.scalar.is_some() && else_arm.result.scalar.is_some();
        let join = match (
            projected_origins,
            then_arm.result.output_points.as_slice(),
            else_arm.result.output_points.as_slice(),
        ) {
            (true, [then_point], [else_point]) => self.point_with_origins(
                CfgPointKind::ConditionalOutput {
                    then_point: *then_point,
                    else_point: *else_point,
                },
                span,
                scope,
                None,
            ),
            _ if joined_value || checked_result => {
                self.point_with_origins(CfgPointKind::Join, span, scope, None)
            }
            _ => self.point(CfgPointKind::Join, span, scope, BTreeSet::new()),
        };
        if matches!(self.cfg.points[join.0].kind, CfgPointKind::Join) {
            let exact_checked_receipt = if checked_result {
                let terminal = |arm: &ReferenceConditionalArm| {
                    let [terminal] = arm.result.output_points.as_slice() else { return None };
                    let stored = &self.cfg.points[terminal.0].possible_origins;
                    match &arm.result.scalar {
                        Some(ReferenceValue::Checked(origins)) if stored.as_ref() == Some(origins) =>
                            Some(*terminal),
                        Some(ReferenceValue::Uncomputed) => Some(*terminal),
                        None if stored.is_none() => Some(*terminal),
                        _ => None,
                    }
                };
                let mut terminals = [terminal(&then_arm), terminal(&else_arm)];
                if terminals[0].is_some() && terminals[0] == terminals[1] {
                    terminals = [None, None];
                }
                self.proof_points.checked_join_values.push((join, terminals));
                terminals.iter().all(Option::is_some)
            } else {
                false
            };
            if joined_value || exact_checked_receipt {
                if let ([then_point], [else_point]) = (
                    then_arm.result.output_points.as_slice(),
                    else_arm.result.output_points.as_slice(),
                ) {
                    self.aggregate_facts.receipt_loan_forwarding.insert(join, vec![*then_point, *else_point]);
                }
            }
        }
        self.cfg.points[then_arm.end.0].successors.push(join);
        self.cfg.points[else_arm.end.0].successors.push(join);
        Ok(if joined_value || checked_result {
            ReferenceValue::Uncomputed
        } else {
            ReferenceValue::Other
        })
    }

    fn projected_conditional(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
        block: &BlockRegion,
        primary: &FinalOutputSlot,
        row: &ConditionalOutputProjection,
    ) -> Result<ReferenceBlockResult, ReferenceAnalysisError> {
        let ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } = expression
        else {
            unreachable!("checked output projection requires a value conditional")
        };
        let region = &self.call_sites.conditionals[&primary.expression];
        let [then_arm, else_arm] = self.conditional_branches(
            condition,
            then_branch,
            Some(else_branch),
            scope,
            region,
            true,
        )?;
        let arity = row.types.len();
        assert!(arity > 1);
        if scope == ReferenceScopeId(0)
            && matches!(row.types.first(), Some(ScalarType::CheckedReference { .. }))
            && row.types.iter().skip(1).all(|ty| aggregate::ordinary_value(ty) || matches!(ty, ScalarType::Enum(_)))
        {
            if let ConditionalRegion::Value { then_block, else_block, .. } = region {
                for (arm, ordinal) in [(&then_arm, *then_block), (&else_arm, *else_block)] {
                    if arm.result.output_points.first().is_some_and(|point| {
                        self.aggregate_facts.local_array_nulls.contains(point)
                    }) {
                        self.aggregate_facts.local_array_failure_arms.insert(arm.end, ordinal);
                    }
                    if self.aggregate_facts.local_array_copy_allocation.is_some()
                        && matches!(&self.call_sites.named_place_roots, NamedPlaceRootFacts::Prepared(_))
                        && matches!(row.types.first(), Some(ScalarType::CheckedReference { inner, .. })
                            if matches!(inner.as_ref(), ScalarType::Struct(id) if
                                self.structs.iter().find(|structure| &structure.id == id).is_some_and(|structure| {
                                    let mut pointers = 0;
                                    structure.fields.iter().all(|field| match &field.ty {
                                        ScalarType::RawPointer(_) => { pointers += 1; true }
                                        ty => aggregate::ordinary_value(ty),
                                    }) && pointers == 1
                                })))
                    {
                        let terminal = match arm.result.output_points.first() {
                            Some(point) if self.aggregate_facts.local_array_nulls.contains(point) =>
                                aggregate::CheckedProducerTerminal::Null(*point),
                            Some(point) => {
                                let output = &self.cfg.points[point.0];
                                match (&output.kind, output.possible_origins.as_ref()) {
                                    (CfgPointKind::Borrow { place, .. }, Some(origins))
                                        if place.projections.is_empty() && origins.len() == 1
                                            && origins.iter().all(|origin| matches!(
                                                &self.cfg.facts[origin.0],
                                                ReferenceOrigin::Fresh { allocation, loan }
                                                    if allocation.source == self.source
                                                        && allocation.function_span == self.function_span
                                                        && loan.origin_place.binding == place.binding
                                            )) =>
                                    {
                                        aggregate::CheckedProducerTerminal::Fresh { borrow: *point, descriptor: place.binding.clone() }
                                    }
                                    _ => aggregate::CheckedProducerTerminal::Unknown,
                                }
                            }
                            _ => aggregate::CheckedProducerTerminal::Unknown,
                        };
                        self.aggregate_facts.checked_producer_arms.push((arm.end, terminal));
                        self.aggregate_result.checked_producer_blocks.insert(arm.end, ordinal.0);
                    }
                }
            }
        }
        assert_eq!(then_arm.result.output_points.len(), arity);
        assert_eq!(else_arm.result.output_points.len(), arity);
        let mut result = ReferenceBlockResult {
            scalar: None,
            output_points: Vec::with_capacity(arity),
        };
        for (output, slot) in block.final_outputs().enumerate() {
            assert_eq!(
                (slot.item, slot.output, slot.expression),
                (primary.item, output, primary.expression),
            );
            if matches!(row.types[output], ScalarType::CheckedReference { .. }) {
                result.scalar = Some(ReferenceValue::Uncomputed);
            }
            let point = self.point_with_origins(
                CfgPointKind::ConditionalOutput {
                    then_point: then_arm.result.output_points[output],
                    else_point: else_arm.result.output_points[output],
                },
                slot.span,
                scope,
                None,
            );
            result.output_points.push(point);
        }
        assert_eq!(result.output_points.len(), arity);
        let first = result.output_points[0];
        self.cfg.points[then_arm.end.0].successors.push(first);
        self.cfg.points[else_arm.end.0].successors.push(first);
        Ok(result)
    }

    pub(super) fn expression(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        self.expression_with_projected_origins(expression, scope, ordinal, false)
    }

    fn expression_with_projected_origins(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
        projected_origins: bool,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let before = self.cursor;
        let value = self.evaluate_expression(expression, scope, ordinal, projected_origins)?;
        if self.capture_call_points && self.cursor != before {
            if let Some(point) = self.cursor {
                self.proof_points.evaluations.insert(ordinal, point);
            }
        }
        if let Some(point) = self.cursor.filter(|point| Some(*point) != before) {
            if matches!(&self.cfg.points[point.0].kind,
                CfgPointKind::ReadPlace { place: ScalarPlace::Field { base, .. }, pointer_point: Some(_), .. }
                    if matches!(base.as_ref(), ScalarPlace::Dereference { .. }))
                && self.call_sites.structure.checked_types.expressions.get(&ordinal)
                    .is_some_and(aggregate::ordinary_value)
            {
                self.aggregate_facts.scalar_receipt_reads.insert(point);
            }
        }
        if matches!(
            expression,
            ScalarExpression::Member { .. }
                | ScalarExpression::Dereference { .. }
                | ScalarExpression::IndexedRead { .. }
        ) && matches!(
            self.call_sites
                .structure
                .checked_types
                .expressions
                .get(&ordinal),
            Some(ScalarType::RawPointer(_) | ScalarType::Struct(_))
        ) {
            if let Some(point) = self.cursor.filter(|point| Some(*point) != before) {
                if matches!(
                    self.cfg.points[point.0].kind,
                    CfgPointKind::ReadPlace { .. }
                ) {
                    let raw = matches!(self.call_sites.structure.checked_types.expressions.get(&ordinal), Some(ScalarType::RawPointer(_)));
                    self.aggregate_facts.projected_reads.insert(point, raw);
                }
            }
        }
        Ok(value)
    }

    fn evaluate_expression(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
        projected_origins: bool,
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
                    projected_origins,
                    matches!(calls.structure.checked_types.expressions.get(&ordinal),
                        Some(ScalarType::CheckedReference { .. })),
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
                self.conditional(condition, then_branch, None, span, scope, region, false, false)
            }
            ScalarExpression::Block(block) => {
                let result = self.block_result(block, self.block_ordinal(parent, OccurrenceEdge::BlockBody), false)?;
                let checked_terminal = match result.output_points.as_slice() {
                    [terminal] if matches!(calls.structure.checked_types.expressions.get(&ordinal),
                        Some(ScalarType::CheckedReference { .. })) => Some(*terminal),
                    _ => None,
                };
                let scalar = result.scalar.or_else(|| {
                    checked_terminal
                        .filter(|terminal| self.cfg.points[terminal.0].possible_origins.is_none())
                        .map(|_| ReferenceValue::Uncomputed)
                });
                let value = match scalar {
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
                if !matches!(value, ReferenceValue::Other) {
                    if let [output] = result.output_points.as_slice() {
                        let wrapper = self.cursor.expect("block value");
                        self.aggregate_facts.receipt_loan_forwarding.insert(wrapper, vec![*output]);
                        if checked_terminal.is_some() {
                            let frozen = &self.cfg.points[wrapper.0].possible_origins;
                            if frozen.is_none() || frozen == &self.cfg.points[output.0].possible_origins {
                                self.proof_points.checked_block_values.push((wrapper, *output));
                            }
                        }
                    }
                }
                Ok(value)
            }
            ScalarExpression::Call { arguments, .. } => {
                let target = self.call_target(expression, ordinal)?;
                let mut address = None;
                let mut checked_release_address = None;
                let mut named_release_binding = None;
                let mut argument_points = Vec::new();
                let signature = calls.checked_call_signatures.get(&ordinal);
                let mut aggregate_arguments = BTreeMap::new();
                let mut array_argument = None;
                let mut raw_arguments = Vec::new();
                for (position, argument) in arguments.iter().enumerate() {
                    let ordinal =
                        self.expression_ordinal(parent, OccurrenceEdge::CallArgument(position));
                    self.expression(argument, scope, ordinal)?;
                    argument_points.push(self.cursor.expect("evaluated call argument"));
                    if position == 0 && target == ResolvedCallTarget::Core(CoreOperationId::Free) {
                        match argument {
                            ScalarExpression::CheckedAddress { .. } => {
                                checked_release_address = self.cursor
                            }
                            ScalarExpression::Name { name, span } => {
                                named_release_binding = match self
                                    .prepared_name_identity(ordinal, *span)?
                                {
                                    Some(PlainNameIdentity::Lexical(binding)) => {
                                        Some(binding.clone())
                                    }
                                    Some(
                                        PlainNameIdentity::Null | PlainNameIdentity::ModuleValue(_),
                                    ) => None,
                                    None => self.names.get(name).cloned(),
                                };
                                if named_release_binding
                                    .as_ref()
                                    .is_some_and(|binding| self.checked.contains(binding))
                                {
                                    checked_release_address = self.cursor;
                                }
                            }
                            _ => {}
                        }
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
                    let read = self.cursor;
                    self.move_value(argument, scope, ordinal)?;
                    let parameter = signature.and_then(|signature| signature.parameters.get(position));
                    if matches!(parameter, Some(ScalarType::RawPointer(_))) {
                        raw_arguments.push(position);
                    }
                    if position == 0 && matches!(parameter, Some(ScalarType::RuntimeArray { .. })) {
                        array_argument = self.cursor.filter(|point| Some(*point) != read)
                            .and_then(|point| match &self.cfg.points[point.0].kind {
                                CfgPointKind::Move { place, ty, owner }
                                    if place.projections.is_empty() && &place.binding == owner
                                        && parameter.is_some_and(|parameter| aggregate::same_runtime_array_type(ty, parameter)) =>
                                    Some((point, owner.clone())),
                                _ => None,
                            });
                    }
                    if matches!(parameter, Some(ScalarType::Struct(_))) {
                        let whole_move = self.cursor.filter(|moved| {
                            Some(*moved) != read && matches!(&self.cfg.points[moved.0].kind,
                                CfgPointKind::Move { place, .. } if place.projections.is_empty())
                        });
                        let input = if let Some(moved) = whole_move {
                            aggregate::ArgumentProvenance::WholeMove(moved)
                        } else if let Some(producer) = read.filter(|point| {
                            matches!(argument, ScalarExpression::Call { .. })
                                && self.aggregate_facts.calls.get(point) == Some(&ordinal)
                        }) {
                            self.aggregate_facts.temporary_arguments.insert(producer);
                            aggregate::ArgumentProvenance::Immediate(producer)
                        } else {
                            aggregate::ArgumentProvenance::Unmoved
                        };
                        aggregate_arguments.insert(position, input);
                    }
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
                        let binding = named_release_binding
                            .ok_or_else(|| self.missing_binding(place_name, *argument_span))?;
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
                                Some(self.pointer_cast_origins(
                                    &arguments[0],
                                    self.expression_ordinal(
                                        parent,
                                        OccurrenceEdge::CallArgument(0),
                                    ),
                                    *mutability,
                                    span,
                                )?)
                            }
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => None,
                };
                let read_call = if self.aggregate_facts.tracks_read_sources() {
                    match (&target, signature) {
                        (ResolvedCallTarget::ModuleCallable { .. }, Some(_)) if !raw_arguments.is_empty() =>
                            Some(aggregate::ReadCall::Direct),
                        (ResolvedCallTarget::Core(CoreOperationId::Load), Some(signature))
                            if raw_arguments.as_slice() == [0] && arguments.len() == 1
                                && matches!(signature.outputs.outputs.as_slice(), [out] if aggregate::ordinary_value(&out.ty)) =>
                            Some(aggregate::ReadCall::Load),
                        (ResolvedCallTarget::Core(CoreOperationId::Offset), Some(signature))
                            if raw_arguments.as_slice() == [0] && arguments.len() == 2
                                && matches!(signature.parameters.as_slice(), [ScalarType::RawPointer(_), ScalarType::I64])
                                && matches!(signature.outputs.outputs.as_slice(), [out] if out.ty == signature.parameters[0]) =>
                            Some(aggregate::ReadCall::Offset),
                        _ => None,
                    }
                } else {
                    None
                };
                if read_call.is_some() {
                    self.aggregate_facts.read_consumers.extend(raw_arguments.iter().map(|&slot| argument_points[slot]));
                }
                let call = CfgPointKind::Call {
                    expression: expression.clone(),
                    output: 0,
                    target,
                    argument_points,
                };
                let carries_struct_result = calls
                    .checked_call_signatures
                    .get(&ordinal)
                    .is_some_and(|signature| {
                        matches!(
                            signature.outputs.outputs.as_slice(),
                            [ScalarOutput {
                                ty: ScalarType::Struct(_),
                                ..
                            }]
                        )
                    });
                let array_call = array_argument.is_some() && matches!(&calls.named_place_roots, NamedPlaceRootFacts::Prepared(_))
                    && matches!(&call, CfgPointKind::Call {
                    target: ResolvedCallTarget::ModuleCallable { .. }, ..
                }) && signature.is_some_and(|signature| {
                    aggregate::array_roundtrip_signature(&signature.parameters, &signature.outputs)
                });
                let neutral_array_call = array_call && matches!(&call, CfgPointKind::Call {
                    target: ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. }, ..
                }) && signature.is_some_and(|signature|
                    aggregate::neutral_array_signature(&signature.parameters, &signature.outputs));
                let checked_call_output = matches!(&call, CfgPointKind::Call {
                    target: ResolvedCallTarget::ModuleCallable { .. }
                        | ResolvedCallTarget::LocalCallable(_)
                        | ResolvedCallTarget::CallableField { .. }, ..
                }) && matches!(signature.and_then(|signature| signature.outputs.outputs.first()),
                    Some(ScalarOutput { ty: ScalarType::CheckedReference { .. }, .. }));
                let id = self.point_with_origins(call, span, scope, cast_origins.clone());
                if checked_call_output {
                    self.proof_points.resolved_checked_call_outputs.push(id);
                }
                if let Some((moved, binding)) = array_argument.filter(|_| array_call) {
                    self.aggregate_facts.array_calls.insert(id, (ordinal, moved, binding));
                    if neutral_array_call {
                        self.aggregate_facts.neutral_array_calls.insert(id);
                    }
                }
                if carries_struct_result {
                    self.aggregate_facts.calls.insert(id, ordinal);
                }
                if !aggregate_arguments.is_empty() {
                    for (&slot, &provenance) in &aggregate_arguments {
                        self.aggregate_result.inputs.entry((id, ordinal.0, slot, provenance)).or_default();
                    }
                    self.aggregate_facts.input_moves.insert(id, (ordinal, aggregate_arguments));
                }
                if !raw_arguments.is_empty() {
                    self.aggregate_facts.raw_calls.insert(id, (raw_arguments, calls.declared_unsafe_calls.contains(&ordinal.0)));
                }
                if let Some(read_call) = read_call {
                    self.aggregate_facts.read_calls.insert(id, read_call);
                }
                if self.capture_call_points {
                    let previous = self.cfg.primary_call_points.insert(ordinal, id);
                    assert!(previous.is_none(), "duplicate CFG call for {ordinal:?}");
                }
                Ok(cast_origins.map_or(ReferenceValue::Uncomputed, ReferenceValue::Checked))
            }
            ScalarExpression::Binary {
                operator,
                left,
                right,
                ..
            } => {
                let mut checked_comparison = None;
                if matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
                    let region = &calls.short_circuits[&ordinal];
                    self.expression(left, scope, region.left)?;
                    self.expression(right, scope, region.right)?;
                } else {
                    let left_ordinal = self.expression_ordinal(parent, OccurrenceEdge::BinaryLeft);
                    let right_ordinal = self.expression_ordinal(parent, OccurrenceEdge::BinaryRight);
                    self.expression(left, scope, left_ordinal)?;
                    let left_point = self.cursor.expect("evaluated left scalar operand");
                    self.expression(right, scope, right_ordinal)?;
                    if self.aggregate_facts.tracks_read_sources()
                        && matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
                    {
                        let right_point = self.cursor.expect("evaluated right scalar operand");
                        for (view, ordinal, null) in [(left_point, left_ordinal, right_point), (right_point, right_ordinal, left_point)] {
                            if self.aggregate_facts.raw_nulls.contains(&null)
                                && matches!(calls.structure.checked_types.expressions.get(&ordinal), Some(ScalarType::RawPointer(_)))
                            {
                                self.aggregate_facts.read_consumers.insert(view);
                            }
                        }
                    }
                    if calls.call_occurrences.len() == 2
                        && self.aggregate_facts.passive_checked_comparisons.len() < 3
                        && matches!(self.proof_points.backing_descriptor_install, ScalarDescriptorCandidates::One(..))
                        && matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
                    {
                        let right_point = self.cursor.expect("evaluated right scalar operand");
                        for (view, view_ordinal, null_expression, null_ordinal) in [
                            (left_point, left_ordinal, right.as_ref(), right_ordinal),
                            (right_point, right_ordinal, left.as_ref(), left_ordinal),
                        ] {
                            if matches!(&self.cfg.points[view.0].kind, CfgPointKind::Read { .. })
                                && matches!(calls.structure.checked_types.expressions.get(&view_ordinal),
                                    Some(ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner })
                                        if aggregate::ordinary_value(inner))
                            {
                                if let ScalarExpression::Name { span, .. } = null_expression {
                                    if matches!(self.prepared_name_identity(null_ordinal, *span)?, Some(PlainNameIdentity::Null)) {
                                        checked_comparison = Some((view, *operator == BinaryOperator::NotEqual));
                                    }
                                }
                            }
                        }
                    }
                }
                let point = self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                if let Some(comparison) = checked_comparison {
                    self.aggregate_facts.passive_checked_comparisons.insert(point, comparison);
                }
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
                let track_raw = !self.aggregate_facts.raw_formals.is_empty() || !self.aggregate_facts.formals.is_empty();
                let mut operands = Vec::new();
                let mut checked_inputs = Vec::new();
                for (position, element) in elements.iter().enumerate() {
                    let ordinal =
                        self.expression_ordinal(parent, OccurrenceEdge::AggregateElement(position));
                    let before = self.cursor;
                    self.expression(element, scope, ordinal)?;
                    if let Some(point) = self.cursor.filter(|point| Some(*point) != before) {
                        match calls.structure.checked_types.expressions.get(&ordinal) {
                            Some(ScalarType::RawPointer(_) | ScalarType::Struct(_)) if track_raw => operands.push(point),
                            Some(ScalarType::CheckedReference { .. }) => checked_inputs.push(point),
                            _ => {}
                        }
                    }
                    self.move_value(element, scope, ordinal)?;
                }
                let point = self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                if !operands.is_empty() {
                    self.aggregate_facts.opaque_container_inputs.insert(point, operands);
                }
                if !checked_inputs.is_empty() {
                    self.aggregate_facts.receipt_loan_sinks.insert(point, checked_inputs);
                }
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Utf8 { .. } => {
                let id = self.point(CfgPointKind::Evaluate(expression.clone()), span, scope, BTreeSet::new());
                if matches!(calls.structure.checked_types.expressions.get(&ordinal), Some(ScalarType::Struct(_))) {
                    self.aggregate_facts.plain_literals.insert(id);
                }
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::StructLiteral { fields, .. } => {
                let mut inputs = Vec::new();
                let mut checked_inputs = Vec::new();
                let structure_id = match calls.structure.checked_types.expressions.get(&ordinal) {
                    Some(ScalarType::Struct(id)) => Some(id),
                    _ => None,
                };
                let mut plain = structure_id.is_some();
                let mut null_view = false;
                let mut carrier = structure_id.is_some();
                let mut raw_fields = 0;
                let mut nullable = structure_id.is_some() && self.aggregate_facts.local_array_copy_allocation.is_some();
                let mut nullable_read = None;
                for (position, field) in fields.iter().enumerate() {
                    let ordinal =
                        self.expression_ordinal(parent, OccurrenceEdge::AggregateField(position));
                    let before = self.cursor;
                    self.expression(&field.value, scope, ordinal)?;
                    let member = calls.structure.literal_fields.get(&ordinal);
                    let member_type = calls.structure.checked_types.expressions.get(&ordinal);
                    if carrier {
                        carrier = member.is_some_and(|field| Some(&field.structure) == structure_id)
                            && member_type.is_some_and(|ty| match ty {
                                ScalarType::RawPointer(_) => { raw_fields += 1; true }
                                ty => aggregate::ordinary_value(ty),
                            });
                    }
                    if nullable {
                        nullable = match (member, member_type, &field.value) {
                            (Some(member), Some(ScalarType::RawPointer(_)), ScalarExpression::Name { span, .. })
                                if Some(&member.structure) == structure_id && nullable_read.is_none() =>
                            {
                                let point = self.cursor.filter(|point| Some(*point) != before);
                                match (point, self.prepared_name_identity(ordinal, *span)?) {
                                    (Some(point), Some(PlainNameIdentity::Lexical(binding)))
                                        if matches!(&self.cfg.points[point.0].kind, CfgPointKind::Read { binding: actual } if actual == binding) =>
                                    {
                                        nullable_read = Some(binding.clone());
                                        true
                                    }
                                    _ => false,
                                }
                            }
                            (Some(member), Some(ty), ScalarExpression::Name { span, .. })
                                if Some(&member.structure) == structure_id && aggregate::ordinary_value(ty) =>
                                matches!(self.prepared_name_identity(ordinal, *span)?, Some(PlainNameIdentity::Lexical(_))),
                            (Some(member), Some(ty), ScalarExpression::Boolean { .. } | ScalarExpression::Char { .. }
                                | ScalarExpression::Integer { .. } | ScalarExpression::Float { .. })
                                if Some(&member.structure) == structure_id => aggregate::ordinary_value(ty),
                            _ => false,
                        };
                    }
                    if plain {
                        plain = match (&field.value, member.filter(|field| Some(&field.structure) == structure_id).and(member_type)) {
                            (ScalarExpression::Name { span, .. }, Some(ScalarType::RawPointer(_)))
                                if matches!(self.prepared_name_identity(ordinal, *span)?, Some(PlainNameIdentity::Null)) => {
                                null_view = true;
                                true
                            }
                            (ScalarExpression::Boolean { .. } | ScalarExpression::Char { .. } |
                             ScalarExpression::Integer { .. } | ScalarExpression::Float { .. }, Some(ty)) => aggregate::ordinary_value(ty),
                            _ => false,
                        };
                    }
                    if matches!(member_type, Some(ScalarType::CheckedReference { .. })) {
                        if let Some(point) = self.cursor.filter(|point| Some(*point) != before) {
                            checked_inputs.push(point);
                        }
                    }
                    if matches!(member_type, Some(ScalarType::RawPointer(_) | ScalarType::Struct(_))) {
                        if member.is_some() {
                            if let Some(point) = self.cursor.filter(|point| Some(*point) != before)
                            {
                                inputs.push((ordinal, point));
                            }
                        }
                    }
                    self.move_value(&field.value, scope, ordinal)?;
                }
                let literal = self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                if !checked_inputs.is_empty() {
                    self.aggregate_facts.receipt_loan_sinks.insert(literal, checked_inputs);
                }
                if let Some(read) = nullable_read.filter(|_| nullable && carrier && raw_fields == 1) {
                    self.aggregate_facts.checked_nullable_literals.insert(literal, read);
                }
                if carrier && raw_fields == 1 {
                    self.aggregate_facts.carrier_literals.insert(literal);
                }
                if plain && null_view {
                    self.aggregate_facts.plain_literals.insert(literal);
                    if self.aggregate_facts.local_array_copy_allocation.is_some() {
                        self.aggregate_facts.checked_backing_free_literals.insert(literal);
                    }
                } else if plain {
                    self.aggregate_facts.scalar_cells.insert(literal);
                } else if !inputs.is_empty() {
                    self.aggregate_facts.literals.insert(literal, inputs);
                }
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
                let place_ordinal = self.place_ordinal(parent, OccurrenceEdge::ExpressionPlace);
                self.place_expressions(place, scope, place_ordinal)?;
                self.check_raw_address_access(place, place_ordinal, span, scope)?;
                let value = self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                let root = self.place_root(place, place_ordinal);
                let exposed = match (&calls.named_place_roots, root.place) {
                    (NamedPlaceRootFacts::Prepared(roots), ScalarPlace::Name { .. }) => {
                        match roots.get(&root.ordinal) {
                            Some(NamedPlaceRootIdentity::Lexical(binding)) => Some(binding.clone()),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                self.proof_points.raw_address_exposures.push((value, exposed));
                if let Some(&pointer) = self.dereference_points.get(&root.ordinal) {
                    self.aggregate_facts.receipt_loan_sinks.insert(value, vec![pointer]);
                } else if let ScalarPlace::Name { span, .. } = root.place {
                    if let Some(PreparedPlaceRoot::Lexical(binding)) =
                        self.prepared_place_root(root.ordinal, *span)?
                    {
                        if self.checked.contains(binding) {
                            self.aggregate_facts.receipt_loan_address_sinks.insert(value, binding.clone());
                        }
                    }
                }
                if !self.aggregate_facts.allocations.is_empty() || !self.aggregate_facts.plain_literals.is_empty()
                    || !self.aggregate_facts.formals.is_empty() || !self.aggregate_facts.raw_formals.is_empty()
                    || self.aggregate_facts.array_formal.is_some() {
                    let root = self.place_root(place, place_ordinal);
                    if let ScalarPlace::Name { span, .. } = root.place {
                        if let Some(PreparedPlaceRoot::Lexical(binding)) =
                            self.prepared_place_root(root.ordinal, *span)?
                        {
                            if !self.aggregate_facts.plain_literals.is_empty() {
                                self.aggregate_facts.plain_aliases.insert(value, binding.clone());
                            }
                            if !self.aggregate_facts.formals.is_empty() || !self.aggregate_facts.raw_formals.is_empty() {
                                self.aggregate_facts.raw_addresses.insert(value, (binding.clone(), matches!(place, ScalarPlace::Name { .. })));
                            }
                            if self.aggregate_facts.array_formal.as_ref() == Some(binding) {
                                self.aggregate_result.array_invalid = true;
                            }
                            if let Some(&base) = self.aggregate_facts.allocations.get(binding) {
                                self.aggregate_facts.views.insert(value, base);
                            }
                        }
                    }
                }
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Member {
                receiver,
                name,
                receiver_span,
                ..
            } => {
                let member = match self.prepared_member_identity(ordinal, span)? {
                    Some(PreparedMemberIdentity::Lexical { binding, field }) => {
                        if !self.binding_types.contains_key(binding) {
                            return Err(self.missing_context(
                                span,
                                ReferenceAnalysisErrorReason::MissingResolvedBinding,
                            ));
                        }
                        let ty = self.checked_member_type(ordinal, span)?;
                        Some((field, CheckedReadSource::PreparedMember { binding, ty }))
                    }
                    Some(PreparedMemberIdentity::Other) => None,
                    None => match self
                        .names
                        .get(receiver)
                        .and_then(|binding| self.binding_types.get(binding))
                    {
                        Some(ScalarType::Struct(id)) => Some((
                            &self.raw_member_field(id, name, span)?.id,
                            CheckedReadSource::SyntheticMember,
                        )),
                        _ => None,
                    },
                };
                if let Some((field, source)) = member {
                    let place = synthetic_member_place(receiver, *receiver_span, field, span);
                    return self.checked_place_read(&place, expression, span, scope, source);
                }
                self.point(
                    CfgPointKind::Evaluate(expression.clone()),
                    span,
                    scope,
                    BTreeSet::new(),
                );
                Ok(ReferenceValue::Other)
            }
            ScalarExpression::Name { name, .. } => {
                let identity = self.prepared_name_identity(ordinal, span)?;
                let canonical_null = matches!(identity, Some(PlainNameIdentity::Null));
                if canonical_null || (identity.is_none() && name == "null")
                {
                    let id = self.intern(ReferenceOrigin::Null { span });
                    if self.independent_checked_query && canonical_null {
                        self.checked_result_origins.insert(id);
                    }
                    let origins = BTreeSet::from([id]);
                    let point = self.point(
                        CfgPointKind::Evaluate(expression.clone()),
                        span,
                        scope,
                        origins.clone(),
                    );
                    if canonical_null {
                        if self.aggregate_facts.tracks_read_sources() {
                            self.aggregate_facts.raw_nulls.insert(point);
                        }
                        if self.aggregate_facts.local_array_copy_allocation.is_some() {
                            self.aggregate_facts.local_array_nulls.insert(point);
                        }
                    }
                    return Ok(ReferenceValue::Checked(origins));
                }
                if matches!(identity, Some(PlainNameIdentity::ModuleValue(_)))
                    || (identity.is_none()
                        && self.module_names.contains(name)
                        && !self.names.contains_key(name))
                {
                    self.point(
                        CfgPointKind::Evaluate(expression.clone()),
                        span,
                        scope,
                        BTreeSet::new(),
                    );
                    return Ok(ReferenceValue::Other);
                }
                let binding = match identity {
                    Some(PlainNameIdentity::Lexical(binding)) => {
                        if !self.binding_types.contains_key(binding) {
                            return Err(self.missing_context(
                                span,
                                ReferenceAnalysisErrorReason::MissingResolvedBinding,
                            ));
                        }
                        binding.clone()
                    }
                    None => self.binding(name, span)?,
                    _ => unreachable!("null and module name reads handled above"),
                };
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
                let mut named_parent_point = None;
                let mut reborrow = None;
                let parents = match root.place {
                    ScalarPlace::Dereference { pointer, .. } => {
                        let pointer_ordinal = self.expression_ordinal(
                            OccurrenceParent::Place(root.ordinal),
                            OccurrenceEdge::DereferencePointer,
                        );
                        let ReferenceValue::Checked(origins) =
                            self.expression(pointer, scope, pointer_ordinal)?
                        else {
                            let binding = match pointer.as_ref() {
                                ScalarExpression::Name { name, span } => {
                                    self.named_expression_binding(name, *span, pointer_ordinal)?
                                }
                                _ => return Err(self.missing_context(
                                    span,
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                )),
                            };
                            if matches!(place, ScalarPlace::Dereference { .. }) {
                                let read = self.cursor.expect("named checked read");
                                let pending = self.proof_points.pending_named_reborrow.as_ref();
                                let nested = pending.filter(|pending| pending.handle != binding
                                    && matches!(self.binding_types.get(&binding),
                                        Some(ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, .. })))
                                    .and_then(|pending| pending.sites.iter().filter(|site| site.parent.is_none())
                                    .find_map(|site| {
                                        let holder = site.holder?;
                                        let point = &self.cfg.points[holder.0];
                                        (holder < read
                                            && matches!(&point.kind, CfgPointKind::Bind { binding: child } if *child == binding))
                                            .then_some((site.point, holder))
                                    }));
                                let direct = if pending.is_none_or(|pending| pending.handle == binding) {
                                    self.deferred_reborrow_roots.iter().find(|(handle, root, entry, _)| {
                                        *handle == binding
                                            && (*mutability == ScalarReferenceMutability::Shared
                                                || matches!(root, super::control::DeferredReborrowRoot::Formal { .. })
                                                || self.cfg.points[entry.0].scope == scope)
                                    })
                                } else {
                                    None
                                };
                                if nested.is_some() || direct.is_some() {
                                    if matches!(&self.cfg.points[read.0].kind,
                                        CfgPointKind::Read { binding: read_binding } if *read_binding == binding)
                                        && self.cfg.points[read.0].possible_origins.is_none()
                                    {
                                        let seed = if pending.is_none() {
                                            direct.map(|(_, root, entry, origin)| (root.clone(), *entry, *origin))
                                        } else {
                                            None
                                        };
                                        let point = self.point_with_origins(
                                            CfgPointKind::Evaluate(expression.clone()), span, scope, None,
                                        );
                                        let pending = self.proof_points.pending_named_reborrow
                                            .get_or_insert_with(|| {
                                                let (root, entry, origin) = seed.expect("initial named reborrow root");
                                                super::control::PendingNamedReborrow {
                                                    sites: Vec::new(), entry, handle: binding, root, origin,
                                                }
                                            });
                                        pending.sites.push(super::control::PendingReborrowSite {
                                            point, read, holder: None, parent: nested,
                                        });
                                        self.aggregate_facts.receipt_loan_sinks.insert(point, vec![read]);
                                        return Ok(ReferenceValue::Uncomputed);
                                    }
                                }
                            }
                            return Err(self.missing_origin(
                                span,
                                binding,
                                ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            ));
                        };
                        reborrow = self.cursor;
                        if matches!(pointer.as_ref(), ScalarExpression::Name { .. })
                            && origins.len() == 1
                        {
                            named_parent_point = reborrow;
                        }
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
                                let binding =
                                    self.named_place_binding(name, *name_span, root.ordinal)?;
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
                    let local_result = self.independent_checked_query && parent.is_none()
                        && *mutability == ScalarReferenceMutability::Shared
                        && loan.origin_place.projections.is_empty()
                        && matches!(place, ScalarPlace::Name { .. })
                        && self.checked_scalar_storage.contains(&loan.origin_place.binding);
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
                    if local_result {
                        self.checked_result_origins.insert(id);
                    }
                    origins.insert(id);
                    let borrow = self.point(
                        CfgPointKind::Borrow {
                            place: loan.origin_place.clone(),
                            loan,
                        },
                        span,
                        scope,
                        BTreeSet::from([id]),
                    );
                    if let (Some(pointer), Some(origin)) = (named_parent_point, parent_id) {
                        self.proof_points.named_reborrow_parents.push(super::control::NamedReborrowParent {
                            borrow,
                            pointer,
                            origin,
                        });
                    }
                }
                if let (Some(operand), Some(value)) = (reborrow, self.cursor) {
                    if value != operand {
                        self.aggregate_facts.receipt_loan_sinks.insert(value, vec![operand]);
                    }
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
        self.place_expressions_with_binding(place, scope, ordinal, None)
    }

    fn check_raw_address_access(
        &mut self,
        place: &ScalarPlace,
        ordinal: OccurrenceOrdinal,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) -> Result<(), ReferenceAnalysisError> {
        let ScalarPlace::Dereference { pointer, .. } = place else {
            return Ok(());
        };
        if !matches!(self.call_sites.plain_names, PlainNameFacts::Prepared(_)) {
            return Ok(());
        }
        let pointer_ordinal = self.expression_ordinal(
            OccurrenceParent::Place(ordinal),
            OccurrenceEdge::DereferencePointer,
        );
        let pointer_type = self
            .call_sites
            .structure
            .checked_types
            .expressions
            .get(&pointer_ordinal)
            .ok_or_else(|| {
                self.missing_context(
                    expression_span(pointer),
                    ReferenceAnalysisErrorReason::MissingCheckedExpressionResult {
                        ordinal: pointer_ordinal.0,
                    },
                )
            })?;
        if matches!(pointer_type, ScalarType::CheckedReference { .. }) {
            self.point(
                CfgPointKind::ReadPlace {
                    place: place.clone(),
                    pointer_point: Some(self.dereference_points[&ordinal]),
                    candidates: Vec::new(),
                    reads_origin: false,
                },
                span,
                scope,
                BTreeSet::new(),
            );
        }
        Ok(())
    }

    fn place_expressions_with_binding(
        &mut self,
        place: &ScalarPlace,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
        root_binding: Option<&ReferenceBindingId>,
    ) -> Result<(), ReferenceAnalysisError> {
        let parent = OccurrenceParent::Place(ordinal);
        match place {
            ScalarPlace::Name { name, span } => {
                if let Some(binding) = root_binding {
                    self.read_checked_place_root(binding, *span, scope);
                } else {
                    match self.prepared_place_root(ordinal, *span)? {
                        Some(PreparedPlaceRoot::Lexical(binding)) => {
                            self.read_checked_place_root(binding, *span, scope);
                        }
                        Some(PreparedPlaceRoot::NonLexical) => {}
                        None => self.place_name(name, *span, scope)?,
                    }
                }
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
            ScalarPlace::Field { base, .. } => self.place_expressions_with_binding(
                base,
                scope,
                self.place_ordinal(parent, OccurrenceEdge::PlaceBase),
                root_binding,
            )?,
            ScalarPlace::Index { base, index, .. } => {
                self.place_expressions_with_binding(
                    base,
                    scope,
                    self.place_ordinal(parent, OccurrenceEdge::PlaceBase),
                    root_binding,
                )?;
                self.expression_child(index, scope, parent, OccurrenceEdge::PlaceIndex)?;
            }
        }
        Ok(())
    }

    fn read_checked_place_root(
        &mut self,
        binding: &ReferenceBindingId,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) {
        if self.checked.contains(binding) {
            self.point(
                CfgPointKind::Read {
                    binding: binding.clone(),
                },
                span,
                scope,
                BTreeSet::new(),
            );
        }
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
        source: CheckedReadSource<'_>,
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
            CheckedReadSource::PreparedMember { binding, .. } => {
                let ScalarPlace::Name { span, .. } = reference_root(place) else {
                    unreachable!("synthetic member receiver")
                };
                if self.checked.contains(binding) {
                    self.point(
                        CfgPointKind::Read {
                            binding: binding.clone(),
                        },
                        *span,
                        scope,
                        BTreeSet::new(),
                    );
                }
                Ok(())
            }
        }
    }

    fn checked_place_read(
        &mut self,
        place: &ScalarPlace,
        expression: &ScalarExpression,
        span: ByteSpan,
        scope: ReferenceScopeId,
        source: CheckedReadSource<'_>,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let canonical_root = match source {
            CheckedReadSource::Place(ordinal) => Some(self.place_root(place, ordinal)),
            _ => None,
        };
        let root = canonical_root
            .as_ref()
            .map_or_else(|| reference_root(place), |root| root.place);
        let prepared = matches!(source, CheckedReadSource::Place(_))
            && matches!(
                self.call_sites.named_place_roots,
                NamedPlaceRootFacts::Prepared(_)
            );
        let mut root_binding = None;
        let module_root = if prepared {
            match self.prepared_read_root(canonical_root.as_ref().unwrap())? {
                Some(PreparedPlaceRoot::Lexical(binding)) => {
                    root_binding = Some(binding);
                    false
                }
                Some(PreparedPlaceRoot::NonLexical) => true,
                None => false,
            }
        } else {
            !matches!(source, CheckedReadSource::PreparedMember { .. })
                && matches!(root, ScalarPlace::Name { name, .. }
                    if self.module_names.contains(name) && !self.names.contains_key(name))
        };
        if module_root {
            self.point(
                CfgPointKind::Evaluate(expression.clone()),
                span,
                scope,
                BTreeSet::new(),
            );
            return Ok(ReferenceValue::Other);
        }
        let reads_checked_value = match source {
            CheckedReadSource::PreparedMember { ty, .. } => {
                matches!(ty, ScalarType::CheckedReference { .. })
            }
            CheckedReadSource::Place(ordinal) if prepared => matches!(
                self.prepared_read_place_type(ordinal, place)?,
                ScalarType::CheckedReference { .. }
            ),
            _ => matches!(
                self.raw_indexed_move_type(place)?,
                ScalarType::CheckedReference { .. }
            ),
        };
        let pointer_type = if let ScalarPlace::Dereference { pointer, .. } = root {
            if prepared {
                self.prepared_read_pointer_type(canonical_root.as_ref().unwrap(), pointer)?
            } else if let ScalarExpression::Name { name, .. } = pointer.as_ref() {
                self.binding_types.get(&self.binding(name, span)?)
            } else {
                None
            }
        } else {
            None
        };
        let checked_pointee = matches!(pointer_type, Some(ScalarType::CheckedReference { .. }));
        let reads_checked_aggregate = matches!(pointer_type,
            Some(ScalarType::CheckedReference { inner, .. })
                if matches!(inner.as_ref(), ScalarType::Struct(_) | ScalarType::Array { .. }));
        if !reads_checked_value
            && !reads_checked_aggregate
            && matches!(root, ScalarPlace::Dereference { pointer, .. } if !matches!(pointer.as_ref(), ScalarExpression::Name { .. }))
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
        if let (CheckedReadSource::Place(ordinal), Some(binding)) = (source, root_binding) {
            self.place_expressions_with_binding(place, scope, ordinal, Some(binding))?;
        } else {
            self.read_place_expressions(place, scope, source)?;
        }
        let pointer_point = match canonical_root {
            None => None,
            Some(root) => match root.place {
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
            },
        };
        let candidates = match (source, root_binding) {
            (CheckedReadSource::PreparedMember { binding, .. }, _) | (_, Some(binding)) => {
                vec![self.reference_place(place, binding.clone(), None)]
            }
            _ if pointer_point.is_none() => vec![self.assignment_place(place, span)?],
            _ => Vec::new(),
        };
        let event = CfgPointKind::ReadPlace {
            place: place.clone(),
            pointer_point,
            candidates,
            reads_origin: reads_checked_value,
        };
        let uncomputed = reads_checked_value || reads_checked_aggregate;
        let point = self.point_with_origins(
            event,
            span,
            scope,
            (!uncomputed).then(BTreeSet::new),
        );
        if checked_pointee && pointer_point.is_some() {
            self.proof_points.checked_pointee_reads.push(point);
        }
        if uncomputed {
            Ok(ReferenceValue::Uncomputed)
        } else {
            Ok(ReferenceValue::Other)
        }
    }

    fn prepared_read_root(
        &self,
        root: &ReferencePlaceRoot<'_>,
    ) -> Result<Option<PreparedPlaceRoot<'a>>, ReferenceAnalysisError> {
        let ScalarPlace::Name { span, .. } = root.place else {
            return Ok(None);
        };
        self.prepared_place_root(root.ordinal, *span)
    }

    fn prepared_place_root(
        &self,
        ordinal: OccurrenceOrdinal,
        span: ByteSpan,
    ) -> Result<Option<PreparedPlaceRoot<'a>>, ReferenceAnalysisError> {
        let NamedPlaceRootFacts::Prepared(roots) = &self.call_sites.named_place_roots else {
            return Ok(None);
        };
        let identity = match roots.get(&ordinal) {
            Some(NamedPlaceRootIdentity::Lexical(binding)) => PreparedPlaceRoot::Lexical(binding),
            Some(NamedPlaceRootIdentity::ModuleValue(_)) => PreparedPlaceRoot::NonLexical,
            None if self
                .call_sites
                .structure
                .resolved_namespace_places
                .contains_key(&ordinal) =>
            {
                PreparedPlaceRoot::NonLexical
            }
            None => {
                return Err(self
                    .missing_context(span, ReferenceAnalysisErrorReason::MissingResolvedBinding))
            }
        };
        Ok(Some(identity))
    }

    fn prepared_read_place_type(
        &self,
        ordinal: OccurrenceOrdinal,
        place: &ScalarPlace,
    ) -> Result<&'a ScalarType, ReferenceAnalysisError> {
        self.call_sites
            .structure
            .checked_types
            .places
            .get(&ordinal)
            .ok_or_else(|| {
                self.missing_context(
                    scalar_place_target_span(place),
                    ReferenceAnalysisErrorReason::MissingCheckedPlaceType { ordinal: ordinal.0 },
                )
            })
    }

    fn prepared_read_pointer_type(
        &self,
        root: &ReferencePlaceRoot<'_>,
        pointer: &ScalarExpression,
    ) -> Result<Option<&'a ScalarType>, ReferenceAnalysisError> {
        let ScalarExpression::Name { name, span } = pointer else {
            // Preserve the inherited non-name dereference gate (direct core.pointer_cast).
            self.raw_indexed_move_type(root.place)?;
            return Ok(None);
        };
        let ordinal = self.expression_ordinal(
            OccurrenceParent::Place(root.ordinal),
            OccurrenceEdge::DereferencePointer,
        );
        if !matches!(
            self.prepared_name_identity(ordinal, *span)?,
            Some(PlainNameIdentity::Lexical(_))
        ) {
            return Err(ReferenceAnalysisError::MissingBinding {
                source_span: SourceSpan::new(
                    self.source.clone(),
                    scalar_place_target_span(root.place),
                ),
                name: name.clone(),
            });
        }
        self.call_sites
            .structure
            .checked_types
            .expressions
            .get(&ordinal)
            .map(Some)
            .ok_or_else(|| {
                self.missing_context(
                    *span,
                    ReferenceAnalysisErrorReason::MissingCheckedExpressionResult {
                        ordinal: ordinal.0,
                    },
                )
            })
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

    fn raw_indexed_move_type(
        &self,
        place: &ScalarPlace,
    ) -> Result<ScalarType, ReferenceAnalysisError> {
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
                let ScalarType::Struct(id) = self.raw_indexed_move_type(base)? else {
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
            ScalarPlace::Index { base, span, .. } => match self.raw_indexed_move_type(base)? {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    Ok(*element)
                }
                _ => Err(self
                    .missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding)),
            },
            ScalarPlace::Dereference { pointer, span } => {
                let pointer_type = match pointer.as_ref() {
                    ScalarExpression::Name { name, .. } => {
                        self.raw_indexed_move_type(&ScalarPlace::Name {
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
        let (binding, place, ty, span) = match value {
            ScalarExpression::Name { name, span } => {
                let binding = match self.prepared_name_identity(ordinal, *span)? {
                    Some(PlainNameIdentity::Lexical(binding)) => Some(binding),
                    Some(PlainNameIdentity::Null | PlainNameIdentity::ModuleValue(_)) => None,
                    None => self.names.get(name),
                };
                let Some(binding) = binding else {
                    return Ok(());
                };
                (
                    binding,
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
                let (binding, field, ty) = match self.prepared_member_identity(ordinal, *span)? {
                    Some(PreparedMemberIdentity::Lexical { binding, field }) => {
                        if !self.binding_types.contains_key(binding) {
                            return Err(self.missing_context(
                                *span,
                                ReferenceAnalysisErrorReason::MissingResolvedBinding,
                            ));
                        }
                        (binding, field, self.checked_member_type(ordinal, *span)?)
                    }
                    Some(PreparedMemberIdentity::Other) => return Ok(()),
                    None => {
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
                        let field = self.raw_member_field(id, name, *span)?;
                        (binding, &field.id, &field.ty)
                    }
                };
                (
                    binding,
                    synthetic_member_place(receiver, *receiver_span, field, *span),
                    ty.clone(),
                    *span,
                )
            }
            ScalarExpression::IndexedRead { place, span } => {
                let ty = if matches!(
                    self.call_sites.named_place_roots,
                    NamedPlaceRootFacts::Prepared(_)
                ) {
                    let ordinal = self.place_ordinal(
                        OccurrenceParent::Expression(ordinal),
                        OccurrenceEdge::ExpressionPlace,
                    );
                    let root = self.place_root(place, ordinal);
                    if matches!(
                        self.prepared_read_root(&root)?,
                        Some(PreparedPlaceRoot::NonLexical)
                    ) {
                        return Ok(());
                    }
                    if let ScalarPlace::Dereference { pointer, .. } = root.place {
                        self.prepared_read_pointer_type(&root, pointer)?;
                    }
                    self.prepared_read_place_type(ordinal, place)?.clone()
                } else {
                    if matches!(reference_root(place), ScalarPlace::Name { name, .. }
                        if self.module_names.contains(name) && !self.names.contains_key(name))
                    {
                        return Ok(());
                    }
                    self.raw_indexed_move_type(place)?
                };
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
                let place_ordinal = self.place_ordinal(
                    OccurrenceParent::Expression(ordinal),
                    OccurrenceEdge::ExpressionPlace,
                );
                let root = self.place_root(place, place_ordinal);
                let ScalarPlace::Dereference { pointer, .. } = root.place else {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                };
                let prepared = matches!(
                    self.call_sites.named_place_roots,
                    NamedPlaceRootFacts::Prepared(_)
                );
                let (ty, prepared_pointer_type) = if prepared {
                    (
                        self.prepared_read_place_type(place_ordinal, place)?.clone(),
                        self.prepared_read_pointer_type(&root, pointer)?,
                    )
                } else {
                    (self.raw_indexed_move_type(place)?, None)
                };
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
                let pointer_type = if prepared {
                    prepared_pointer_type
                } else if let ScalarExpression::Name { name, .. } = pointer.as_ref() {
                    let binding = self.binding(name, *span)?;
                    self.binding_types.get(&binding)
                } else {
                    None
                };
                if !matches!(
                    pointer_type,
                    Some(ScalarType::CheckedReference {
                        mutability: ScalarReferenceMutability::Mutable,
                        ..
                    })
                ) {
                    return Err(self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    ));
                }
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
        let binding = binding.clone();
        let place = self.reference_place(&place, binding.clone(), None);
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

    fn project_call_output(
        &mut self,
        producer: CfgPointId,
        ordinal: OccurrenceOrdinal,
        output: usize,
    ) -> CfgPointId {
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
        let checked_call_output = matches!(target, ResolvedCallTarget::ModuleCallable { .. }
            | ResolvedCallTarget::LocalCallable(_) | ResolvedCallTarget::CallableField { .. })
            && self.call_sites.checked_call_signatures.get(&ordinal)
                .and_then(|signature| signature.outputs.outputs.get(output))
                .is_some_and(|slot| matches!(slot.ty, ScalarType::CheckedReference { .. }));
        let kind = CfgPointKind::Call {
            expression: expression.clone(),
            output,
            target: target.clone(),
            argument_points: argument_points.clone(),
        };
        let projection = self.point_with_origins(kind, point.source_span.range, point.scope, None);
        if checked_call_output {
            self.proof_points.resolved_checked_call_outputs.push(projection);
        }
        if self.aggregate_facts.neutral_array_calls.contains(&producer) {
            self.aggregate_facts.neutral_array_projections.insert(projection, producer);
        }
        projection
    }

    fn additional_receivers(
        &mut self,
        binding: &ScalarBinding,
        block: &ScalarBlock,
        scope: ReferenceScopeId,
        declared: &mut Vec<(String, ReferenceBindingId, Option<ReferenceBindingId>)>,
        call_producer: Option<CfgPointId>,
        receivers: &[BindingReceiverSlot],
        primary: OccurrenceOrdinal,
        secondary: &[BindingOutput],
        position: BlockPosition,
    ) -> Result<(), ReferenceAnalysisError> {
        for receiver in &receivers[1..] {
            let before = self.cursor;
            match &secondary[receiver.position - 1] {
                BindingOutput::Evaluate {
                    position: source,
                    expression,
                } => {
                    self.expression(&binding.output_values[*source].value, scope, *expression)?;
                }
                BindingOutput::ProjectPrimary { output } => {
                    if let Some(producer) = call_producer {
                        self.project_call_output(producer, primary, *output);
                    }
                }
            }
            let initializer = self.cursor.filter(|_| self.cursor != before);
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
            self.record_binding_definition(position, receiver.position, false, initializer);
        }
        Ok(())
    }


    fn checked_backing_forward(&self, region: &BlockRegion, output_points: &[CfgPointId])
        -> Option<(OccurrenceOrdinal, CfgPointId)>
    {
        let calls = self.call_sites;
        let [first, result, status] = region.items.as_slice() else { return None };
        let (BlockItemKind::Binding(binding), BlockItemKind::FinalOutput(left), BlockItemKind::FinalOutput(right)) =
            (&first.kind, &result.kind, &status.kind) else { return None };
        let BindingInitialization::Outputs(outputs) = &binding.initialization else { return None };
        if !matches!(outputs.secondary.as_slice(), [BindingOutput::ProjectPrimary { output: 1 }])
            || !matches!(binding.receivers(), [left, right]
                if matches!(left.ty, ScalarType::CheckedReference { .. }) && aggregate::ordinary_value(&right.ty))
            || !matches!((left.output, right.output), (0, 1))
            || calls.boundary.root_outputs.len() != 2
            || !calls.boundary.parameters.iter().all(|parameter| aggregate::ordinary_value(&parameter.ty))
            || !calls.structure.module_storage_uses.is_empty()
            || !matches!(calls.call_occurrences.as_slice(), [occurrence] if occurrence.ordinal == outputs.primary)
            || !calls.allocation_length_call_targets.is_empty()
            || !calls.callable_escapes.is_empty()
        {
            return None;
        }
        let [read_left, read_right] = output_points else { return None };
        let bound = |lane, read: CfgPointId| {
            let BindingDefinition::Initialized { bind, value } =
                *self.proof_points.definitions.get(&(calls.structure.root, first.position, lane))?
                else { return None };
            let CfgPointKind::Bind { binding } = &self.cfg.points.get(bind.0)?.kind else { return None };
            matches!(&self.cfg.points.get(read.0)?.kind, CfgPointKind::Read { binding: actual } if actual == binding)
                .then_some(value)
        };
        let primary = bound(0, *read_left)?;
        let secondary = bound(1, *read_right)?;
        if self.proof_points.evaluations.get(&outputs.primary) != Some(&primary) {
            return None;
        }
        let CfgPointKind::Call { output: 0, target: primary_target @ ResolvedCallTarget::ModuleCallable {
            concrete: ConcreteCallSelection::Declaration, ..
        }, argument_points, .. } = &self.cfg.points.get(primary.0)?.kind else { return None };
        if !matches!(&self.cfg.points.get(secondary.0)?.kind, CfgPointKind::Call { output: 1, target, argument_points: args, .. }
            if target == primary_target && args == argument_points)
            || argument_points.len() != calls.boundary.parameters.len()
            || !argument_points.iter().enumerate().all(|(slot, point)| matches!(&self.cfg.points[point.0].kind,
                CfgPointKind::Read { binding } if self.proof_points.formals.get(&slot) == Some(binding)))
        {
            return None;
        }
        Some((outputs.primary, primary))
    }

    fn direct_local_reborrow_root(
        &self,
        binding: &ReferenceBindingId,
        ty: &ScalarType,
        bind: CfgPointId,
        value: CfgPointId,
    ) -> Option<(super::control::DeferredReborrowRoot, ReferenceOriginId)> {
        if !self.deferred_local_reborrow || !matches!(ty,
            ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner }
                if aggregate::ordinary_value(inner))
        {
            return None;
        }
        let origins = self.cfg.points[bind.0].possible_origins.as_ref()?;
        let origin = *origins.first().filter(|_| origins.len() == 1)?;
        let source = &self.cfg.points[value.0];
        let CfgPointKind::Borrow { place, loan } = &source.kind else { return None };
        let owner = &place.binding;
        if loan.parent.is_some() || loan.mode != ScalarReferenceMutability::Mutable
            || !self.binding_types.get(owner).is_some_and(aggregate::ordinary_value)
            || self.call_sites.boundary.parameters.iter().any(|formal| formal.span == owner.declaration_span)
            || !place.projections.is_empty()
            || !matches!(&place.place, ScalarPlace::Name { name, .. } if self.names.get(name) == Some(owner))
            || owner.block_span != binding.block_span || source.scope != self.cfg.points[bind.0].scope
            || !matches!(self.cfg.facts.get(origin.0), Some(ReferenceOrigin::Fresh { loan: fact, .. }) if fact == loan)
        {
            return None;
        }
        Some((super::control::DeferredReborrowRoot::Local { owner: owner.clone(), borrow: value }, origin))
    }

    fn record_binding_definition(
        &mut self,
        position: BlockPosition,
        receiver: usize,
        allocation: bool,
        initializer: Option<CfgPointId>,
    ) {
        let bind = self.cursor.expect("existing binding definition point");
        if let CfgPointKind::Bind { binding } = &self.cfg.points[bind.0].kind {
            if let Some(ty) = self.binding_types.get(binding) {
                if !allocation && matches!(ty, ScalarType::CheckedReference { .. }) {
                    if let Some(value) = initializer {
                        let frozen = &self.cfg.points[bind.0].possible_origins;
                        if frozen.is_none()
                            || frozen == &self.cfg.points[value.0].possible_origins
                        {
                            self.proof_points.checked_binding_initializers.push((bind, value));
                            if receiver == 0 && matches!(ty,
                                ScalarType::CheckedReference { inner, .. }
                                    if aggregate::ordinary_value(inner))
                            {
                                if let Some(site) = self.proof_points.pending_named_reborrow.as_mut()
                                    .and_then(|pending| pending.sites.last_mut())
                                    .filter(|site| site.point == value)
                                {
                                    site.holder = Some(bind);
                                }
                            }
                            if let Some((root, origin)) = self.direct_local_reborrow_root(binding, ty, bind, value) {
                                self.deferred_reborrow_roots.push((binding.clone(), root, bind, origin));
                            }
                        }
                    }
                }
                if self.independent_checked_query && !allocation && initializer.is_some() && aggregate::ordinary_value(ty) {
                    self.checked_scalar_storage.insert(binding.clone());
                }
                if (self.aggregate_facts.array_formal.is_some()
                    || self.aggregate_facts.local_array_copy_allocation.is_some())
                    && aggregate::array_neutral_scalar(ty)
                {
                    self.aggregate_facts.neutral_array_scalars.insert(binding.clone());
                }
                self.aggregate_facts.observe_binding(
                    &self.cfg,
                    bind,
                    initializer,
                    allocation,
                    ty,
                    position,
                );
            }
        }
        if self.capture_call_points {
            let definition = if allocation {
                BindingDefinition::Allocation { bind }
            } else if let Some(value) = initializer {
                BindingDefinition::Initialized { bind, value }
            } else {
                return;
            };
            self.proof_points
                .definitions
                .insert((position.block, position.item, receiver), definition);
        }
    }

    fn record_frozen_install(&mut self, destination: OccurrenceOrdinal, rhs: CfgPointId) {
        if self.capture_call_points {
            let install = self.cursor.expect("existing assignment install point");
            self.proof_points
                .installs
                .insert(destination, FrozenInstall { rhs, install });
        }
    }

    fn assignment_destination(
        &mut self,
        target: &ScalarAssignmentTarget,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
    ) -> Result<ReferenceAssignmentDestination<'a>, ReferenceAnalysisError> {
        if let ScalarPlace::Name { span, .. } = &target.place {
            return match &self.call_sites.direct_destinations {
                DirectDestinationFacts::Raw => Ok(ReferenceAssignmentDestination::RawName),
                DirectDestinationFacts::Prepared(rows) => rows
                    .get(&ordinal)
                    .map(ReferenceAssignmentDestination::PreparedName)
                    .ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    }),
            };
        }
        let root = self.place_root(&target.place, ordinal);
        if let ScalarPlace::Name { name, span } = root.place {
            let missing_root = || {
                self.missing_context(*span, ReferenceAnalysisErrorReason::MissingResolvedBinding)
            };
            let binding = match &self.call_sites.named_place_roots {
                NamedPlaceRootFacts::Prepared(roots) => match roots.get(&root.ordinal) {
                    Some(NamedPlaceRootIdentity::Lexical(binding)) => Some(binding.clone()),
                    Some(NamedPlaceRootIdentity::ModuleValue(_)) => None,
                    None if self
                        .call_sites
                        .structure
                        .resolved_namespace_places
                        .contains_key(&root.ordinal) =>
                    {
                        None
                    }
                    None => return Err(missing_root()),
                },
                NamedPlaceRootFacts::Raw => match self.names.get(name) {
                    Some(binding) => Some(binding.clone()),
                    None if self.module_names.contains(name) => None,
                    None => return Err(missing_root()),
                },
            };
            if let Some(binding) = binding {
                self.place_expressions_with_binding(&target.place, scope, ordinal, Some(&binding))?;
                return Ok(ReferenceAssignmentDestination::Resolved(
                    self.reference_place(&target.place, binding, None),
                ));
            }
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
        self.place_expressions(&target.place, scope, ordinal)?;
        let pointer_point = *self.dereference_points.get(&root.ordinal).ok_or_else(|| {
            self.missing_context(
                target.target_span,
                ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
            )
        })?;
        Ok(ReferenceAssignmentDestination::Through(pointer_point))
    }

    fn block(
        &mut self,
        block: &ScalarBlock,
        ordinal: BlockOrdinal,
    ) -> Result<Option<ReferenceValue>, ReferenceAnalysisError> {
        Ok(self.block_result(block, ordinal, false)?.scalar)
    }

    fn block_result(
        &mut self,
        block: &ScalarBlock,
        ordinal: BlockOrdinal,
        projected_origins: bool,
    ) -> Result<ReferenceBlockResult, ReferenceAnalysisError> {
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
        let mut result = ReferenceBlockResult::default();
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
                    let outputs = match &binding_region.initialization {
                        BindingInitialization::Outputs(outputs) => outputs,
                        BindingInitialization::Allocation { .. } => {
                            &calls.query_allocation_outputs[&(ordinal, item_index)]
                        }
                    };
                    let primary = &binding.value;
                    let primary_ordinal = outputs.primary;
                    let position = BlockPosition {
                        block: ordinal,
                        item: item_index,
                    };
                    let (raw_null, prepared_raw_null) = match primary {
                        ScalarExpression::Name { name, span }
                            if matches!(receiver.ty, ScalarType::RawPointer(_)) =>
                        {
                            match self.prepared_name_identity(primary_ordinal, *span)? {
                                Some(PlainNameIdentity::Null) => (true, true),
                                Some(_) => (false, false),
                                None => (name == "null", false),
                            }
                        }
                        _ => (false, false),
                    };
                    let before = self.cursor;
                    let value = if raw_null {
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
                    let initializer = self.cursor.filter(|_| self.cursor != before);
                    if prepared_raw_null
                        && (self.aggregate_facts.local_array_copy_allocation.is_some()
                            || (scope == ReferenceScopeId(0) && calls.call_occurrences.len() == 2
                                && matches!(self.proof_points.backing_descriptor_install,
                                    ScalarDescriptorCandidates::One(..))))
                    {
                        self.aggregate_facts.checked_raw_null_initializers.insert(
                            initializer.expect("prepared raw null evaluation"),
                        );
                    }
                    if self.capture_call_points && raw_null {
                        self.proof_points
                            .evaluations
                            .insert(primary_ordinal, initializer.expect("raw null evaluation"));
                    }
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
                            self.record_binding_definition(
                                position,
                                0,
                                binding.is_allocation,
                                initializer,
                            );
                            let old = self.names.insert(receiver.name.clone(), id.clone());
                            declared.push((receiver.name.clone(), id, old));
                            self.additional_receivers(
                                binding,
                                block,
                                scope,
                                &mut declared,
                                call_producer,
                                binding_region.receivers(),
                                outputs.primary,
                                &outputs.secondary,
                                position,
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
                    self.record_binding_definition(position, 0, binding.is_allocation, initializer);
                    declared.push((receiver.name.clone(), id, old));
                    self.additional_receivers(
                        binding,
                        block,
                        scope,
                        &mut declared,
                        call_producer,
                        binding_region.receivers(),
                        outputs.primary,
                        &outputs.secondary,
                        position,
                    )?;
                }
                (
                    ScalarBlockItem::Expression(expression),
                    kind @ (BlockItemKind::Evaluation { .. } | BlockItemKind::FinalOutput(_)),
                ) => {
                    let ordinal = match kind {
                        BlockItemKind::Evaluation { expression } => *expression,
                        BlockItemKind::FinalOutput(slot) => {
                            if let Some(row) = calls
                                .structure
                                .conditional_output_projections
                                .get(&slot.expression)
                            {
                                result = self
                                    .projected_conditional(expression, scope, region, slot, row)?;
                                continue;
                            }
                            slot.expression
                        }
                        _ => unreachable!("expression item region"),
                    };
                    let value = self.expression_with_projected_origins(
                        expression,
                        scope,
                        ordinal,
                        projected_origins && matches!(kind, BlockItemKind::FinalOutput(_)),
                    )?;
                    let value_point = self.cursor.expect("evaluated output expression");
                    self.move_value(expression, scope, ordinal)?;
                    if let BlockItemKind::FinalOutput(slot) = kind {
                        if scope == ReferenceScopeId(0) && slot.output == 0
                            && self.aggregate_facts.array_formal.is_some()
                        {
                            self.aggregate_facts.array_return_move = self.cursor.filter(|moved| *moved != value_point);
                        }
                        let ty = if scope == ReferenceScopeId(0) {
                            &calls.boundary.root_outputs[slot.output].ty
                        } else {
                            &block.final_output_values[slot.output].ty
                        };
                        result.output_points.push(value_point);
                        if matches!(value, ReferenceValue::Checked(_)) {
                            result.scalar = Some(value);
                        } else if matches!(value, ReferenceValue::Uncomputed)
                            && matches!(ty, ScalarType::CheckedReference { .. })
                        {
                            result.scalar = Some(value);
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
                        let rhs_point = self.project_call_output(producer, projection.producer.expression, projection.output);
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
                    let mut array_rebind = None;
                    let mut simple_destinations = assignment.values.len() == 1
                        && values.first().is_some_and(|(_, rhs)| self.aggregate_facts.array_calls.contains_key(rhs));
                    for ((slot, (value, rhs_point)), destination) in
                        group.installs().zip(values).zip(destinations)
                    {
                        simple_destinations &= matches!(&destination, ReferenceAssignmentDestination::PreparedName(DirectDestinationIdentity::Existing(_)));
                        let position = slot.position;
                        let target = &assignment.targets[position];
                        if matches!(&destination,
                                ReferenceAssignmentDestination::PreparedName(DirectDestinationIdentity::Module(_))
                                | ReferenceAssignmentDestination::EvaluationOnly
                                | ReferenceAssignmentDestination::RawName)
                        {
                            self.proof_points.has_nonlexical_install = true;
                            if self.aggregate_facts.local_array_copy_allocation.is_some() {
                                self.aggregate_facts.checked_producer_nonlocal_installs.insert(rhs_point);
                            }
                        }
                        if let ScalarPlace::Name { name, span } = &target.place {
                            let lexical = match destination {
                                ReferenceAssignmentDestination::PreparedName(identity) => {
                                    match identity {
                                        DirectDestinationIdentity::Existing(binding) => {
                                            Some((binding.clone(), None))
                                        }
                                        DirectDestinationIdentity::Introduce { binding, name } => {
                                            Some((binding.clone(), Some(name.as_str())))
                                        }
                                        DirectDestinationIdentity::Module(_) => None,
                                    }
                                }
                                ReferenceAssignmentDestination::RawName => {
                                    match self.names.get(name) {
                                        Some(binding) => Some((binding.clone(), None)),
                                        None if self.module_names.contains(name) => None,
                                        None => Some((
                                            ReferenceBindingId {
                                                source: self.source.clone(),
                                                function_span: self.function_span,
                                                declaration_span: target.target_span,
                                                block_span: block.span,
                                                kind: ReferenceBindingKind::Declared,
                                            },
                                            Some(name.as_str()),
                                        )),
                                    }
                                }
                                _ => unreachable!("staged direct assignment destination"),
                            };
                            let Some((id, introduction)) = lexical else {
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
                                self.record_frozen_install(slot.destination, rhs_point);
                                continue;
                            };
                            if let Some(name) = introduction {
                                self.names.insert(name.to_owned(), id.clone());
                                let ty = output_types.get(position).ok_or_else(|| {
                                    self.missing_context(
                                        target.target_span,
                                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                    )
                                })?;
                                self.binding_types.insert(id.clone(), ty.clone());
                                declared.push((name.to_owned(), id.clone(), None));
                            }
                            let target_place =
                                self.reference_place(&target.place, id.clone(), None);
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
                                if capture.is_none() && self.proof_points.pending_named_reborrow.is_some()
                                    && matches!(target.place, ScalarPlace::Dereference { ref pointer, .. }
                                        if matches!(pointer.as_ref(), ScalarExpression::Name { .. }))
                                    && matches!(&self.cfg.points[pointer_point.0].kind,
                                        CfgPointKind::Read { binding } if matches!(self.binding_types.get(binding),
                                            Some(ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner })
                                                if aggregate::ordinary_value(inner) && **inner == output_types[position]))
                                {
                                    self.proof_points.deferred_child_scalar_stores
                                        .push(self.cursor.expect("typed scalar store"));
                                }
                            } else {
                                let target_place = match destination {
                                    ReferenceAssignmentDestination::EvaluationOnly => continue,
                                    ReferenceAssignmentDestination::Resolved(place) => place,
                                    ReferenceAssignmentDestination::RawName
                                    | ReferenceAssignmentDestination::PreparedName(_)
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
                        let install = self.cursor.expect("assignment install point");
                        if simple_destinations && position == 0 && self.aggregate_facts.array_calls.get(&rhs_point)
                            .is_some_and(|(_, _, binding)| matches!(&self.cfg.points[install.0].kind,
                                CfgPointKind::Assign { target, rhs_point: actual, .. }
                                    if *actual == rhs_point && target.projections.is_empty() && target.binding == *binding))
                        {
                            array_rebind = Some(install);
                        }
                        self.record_frozen_install(slot.destination, rhs_point);
                        if let Some(capture) = capture.filter(|_| {
                            self.capture_call_points && scope == ReferenceScopeId(0)
                                && assignment.values.len() == 1
                                && (!matches!(self.proof_points.scalar_descriptor_install, ScalarDescriptorCandidates::Multiple)
                                    || !matches!(self.proof_points.backing_descriptor_install, ScalarDescriptorCandidates::Multiple))
                                && matches!(&calls.named_place_roots, NamedPlaceRootFacts::Prepared(_))
                        }) {
                            if let (
                                ScalarType::CheckedReference { inner, .. },
                                CfgPointKind::Assign { target, .. },
                                CfgPointKind::Call { output, target: ResolvedCallTarget::ModuleCallable { .. }, .. },
                                Some(evaluation),
                            ) = (
                                &output_types[position],
                                &self.cfg.points[install.0].kind,
                                &self.cfg.points[rhs_point.0].kind,
                                group.rhs_evaluations().next(),
                            ) {
                                if let ScalarType::Struct(id) = inner.as_ref() {
                                    if target.projections.is_empty() && *output == position
                                        && calls.boundary.root_outputs.iter().all(|output| {
                                            aggregate::ordinary_value(&output.ty) || matches!(output.ty, ScalarType::Unit | ScalarType::Enum(_))
                                        })
                                    {
                                        let known = [&self.proof_points.scalar_descriptor_install, &self.proof_points.backing_descriptor_install]
                                            .into_iter().find_map(|candidate| match candidate {
                                                ScalarDescriptorCandidates::One(_, marker) if &marker.structure == id => Some(marker.validate_backing),
                                                _ => None,
                                            });
                                        let class = known.or_else(|| {
                                            let declaration = self.structs.iter().find(|declaration| &declaration.id == id)?;
                                            if declaration.fields.is_empty() {
                                                return None;
                                            }
                                            let mut fields = 0;
                                            let supported = declaration.fields.iter().all(|field| match &field.ty {
                                                ScalarType::RawPointer(_) => {
                                                    fields += 1;
                                                    fields == 1
                                                }
                                                ty => aggregate::ordinary_value(ty) || matches!(ty, ScalarType::Enum(_)),
                                            });
                                            (supported && (fields == 0 || *output == 0)).then_some(fields == 1)
                                        });
                                        if let Some(validate_backing) = class.filter(|class| !*class || *output == 0) {
                                            let candidate = if validate_backing {
                                                &mut self.proof_points.backing_descriptor_install
                                            } else {
                                                &mut self.proof_points.scalar_descriptor_install
                                            };
                                            match candidate {
                                                ScalarDescriptorCandidates::Multiple => {}
                                                ScalarDescriptorCandidates::One(previous, _) if *previous != slot.destination => {
                                                    *candidate = ScalarDescriptorCandidates::Multiple;
                                                }
                                                _ => {
                                                    let marker = ScalarDescriptorInstall {
                                                        validate_backing,
                                                        frozen: FrozenInstall { rhs: rhs_point, install },
                                                        capture,
                                                        binding: target.binding.clone(),
                                                        call: evaluation.expression,
                                                        output: *output,
                                                        structure: id.clone(),
                                                    };
                                                    if let ScalarDescriptorCandidates::One(_, previous) = candidate {
                                                        **previous = marker;
                                                    } else {
                                                        *candidate = ScalarDescriptorCandidates::One(slot.destination, Box::new(marker));
                                                    }
                                                }
                                            }
                                        }
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
                    if let Some(install) = array_rebind.filter(|_| simple_destinations) {
                        self.aggregate_facts.array_installs.insert(install);
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
                _ => panic!(
                    "mismatched block item {ordinal:?}/{item_index} in {:?}",
                    self.source
                ),
            }
        }
        if scope == ReferenceScopeId(0) {
            let backing_forward = self.checked_backing_forward(region, &result.output_points);
            for (slot, &value_point) in region.final_outputs().zip(&result.output_points) {
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
                if let Some((ordinal, primary)) = backing_forward.filter(|_| slot.output == 0) {
                    self.aggregate_result.checked_backing_forward_candidate =
                        Some((ordinal, primary, self.cursor.expect("checked backing terminal output")));
                }
            }
        }
        self.point(CfgPointKind::ScopeExit, region.span, scope, BTreeSet::new());
        if self.aggregate_facts.observes_values() {
            self.aggregate_facts.scope_exits.insert(self.cursor.expect("scope exit"), ordinal);
        }
        if scope == ReferenceScopeId(0) && region.final_outputs().next().is_some() {
            let return_span = ByteSpan::new(
                region.final_outputs().next().expect("output").span.start,
                region.final_outputs().last().expect("output").span.end,
            );
            if let Some(value) = &result.scalar {
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
    aggregate: Option<aggregate::AggregateEdgeState>,
    bindings: BTreeMap<ReferenceBindingId, BTreeSet<ReferenceOriginId>>,
    projected: Vec<ReferenceProjectedState>,
    uninitialized: HashSet<ReferenceBindingId>,
    value: Option<BTreeSet<ReferenceOriginId>>,
    released: BTreeSet<ReferenceBindingId>,
    invalidated: BTreeSet<ReferenceBindingId>,
    point_values: ReferencePointValues,
    return_outputs: BTreeMap<usize, BTreeSet<ReferenceOriginId>>,
    moved: Vec<(ReferencePlaceId, SourceSpan)>,
    moved_checked_handles: BTreeMap<ReferenceBindingId, Vec<(Vec<ReferencePlaceProjection>, SourceSpan)>>,
    assignment_previous: BTreeMap<CfgPointId, BTreeSet<ReferenceOriginId>>,
}

#[derive(Clone, Default, PartialEq)]
struct ReferencePointValues(Vec<(CfgPointId, BTreeSet<ReferenceOriginId>)>);

impl ReferencePointValues {
    fn position(&self, point: CfgPointId) -> Result<usize, usize> {
        self.0.binary_search_by_key(&point, |(index, _)| *index)
    }

    fn get(&self, point: CfgPointId) -> Option<&BTreeSet<ReferenceOriginId>> {
        self.position(point).ok().map(|index| &self.0[index].1)
    }

    fn record(
        &mut self,
        point: CfgPointId,
        origins: &Option<BTreeSet<ReferenceOriginId>>,
        last_use: &[usize],
    ) {
        self.0
            .retain(|(index, _)| index.0 > point.0 || last_use[index.0] > point.0);
        if last_use[point.0] > point.0 {
            match (self.position(point), origins) {
                (Ok(index), Some(origins)) => self.0[index].1 = origins.clone(),
                (Err(index), Some(origins)) => self.0.insert(index, (point, origins.clone())),
                (Ok(index), None) => {
                    self.0.remove(index);
                }
                (Err(_), None) => {}
            }
        }
    }

    fn clear_from(&mut self, point: CfgPointId) {
        let retained = self.0.partition_point(|(index, _)| *index < point);
        self.0.truncate(retained);
    }
}

#[cfg(test)]
#[test]
fn sparse_reference_point_values_keep_path_presence_and_expire_at_original_boundaries() {
    let uses = [4, 3, 2, 6, 4, 6, 6];
    let origins = BTreeSet::from([ReferenceOriginId(0)]);
    let empty = BTreeSet::new();
    let mut first = ReferencePointValues::default();
    first.record(CfgPointId(0), &Some(origins.clone()), &uses);
    first.record(CfgPointId(1), &Some(empty.clone()), &uses);
    first.record(CfgPointId(2), &Some(empty.clone()), &uses);
    assert_eq!(first.get(CfgPointId(0)), Some(&origins));
    assert_eq!(first.get(CfgPointId(1)), Some(&empty));
    assert_eq!(first.get(CfgPointId(2)), None);

    let mut sibling = first.clone();
    sibling.record(CfgPointId(3), &Some(origins.clone()), &uses);
    assert_eq!(first.get(CfgPointId(1)), Some(&empty));
    assert_eq!(sibling.get(CfgPointId(1)), None);
    sibling.record(CfgPointId(1), &Some(empty.clone()), &uses);
    assert_eq!(sibling.get(CfgPointId(3)), Some(&origins));
    assert_eq!(sibling.get(CfgPointId(1)), Some(&empty));
    sibling.clear_from(CfgPointId(3));
    assert_eq!(sibling.get(CfgPointId(3)), None);
    assert!(sibling == first);

    sibling.record(CfgPointId(1), &None, &uses);
    assert_eq!(sibling.get(CfgPointId(1)), None);
    sibling.record(CfgPointId(4), &None, &uses);
    assert_eq!(sibling.get(CfgPointId(0)), None);
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

fn is_lexical_field_path(path: &[ReferencePlaceProjection]) -> bool {
    path.iter().all(|projection| {
        matches!(projection, ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(_)))
    })
}

#[derive(Clone, Copy)]
enum CheckedHandleStorage {
    Lexical,
    Pointee,
}

fn unavailable_checked_handles(
    state: &ReferenceEdgeState,
    binding: &ReferenceBindingId,
    path: &[ReferencePlaceProjection],
    storage: CheckedHandleStorage,
) -> Vec<SourceSpan> {
    state
        .moved_checked_handles
        .get(binding)
        .into_iter()
        .flatten()
        .filter(|(moved, _)| {
            (matches!(storage, CheckedHandleStorage::Lexical) || !moved.is_empty())
                && (moved.starts_with(path) || path.starts_with(moved))
        })
        .map(|(_, origin)| origin.clone())
        .collect()
}

fn reinitialize_checked_handles(
    state: &mut ReferenceEdgeState,
    target: &ReferencePlaceId,
    storage: CheckedHandleStorage,
) {
    if target.projections.is_empty() && matches!(storage, CheckedHandleStorage::Lexical) {
        state.moved_checked_handles.remove(&target.binding);
    } else if let Some(moved) = state.moved_checked_handles.get_mut(&target.binding) {
        if is_lexical_field_path(&target.projections) {
            moved.retain(|(path, _)| {
                (matches!(storage, CheckedHandleStorage::Pointee) && path.is_empty())
                    || !path.starts_with(&target.projections)
            });
            if moved.is_empty() {
                state.moved_checked_handles.remove(&target.binding);
            }
        }
    }
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

fn loan_has_ancestor(loan: &ReferenceLoanId, ancestor: &ReferenceLoanId) -> bool {
    let mut next = loan.parent.as_deref();
    while let Some(parent) = next {
        if parent == ancestor {
            return true;
        }
        next = parent.parent.as_deref();
    }
    false
}

enum LoanAccess<'a> {
    Direct,
    Through(ReferenceOriginId),
    WriteThrough(&'a InvalidationCandidate),
    ReadThrough(&'a ReferenceLoanId),
    Borrow(&'a ReferenceLoanId),
}

fn conflicting_live_loan(
    points: &[ReferenceFlowPoint],
    facts: &[ReferenceOrigin],
    state: &ReferenceEdgeState,
    point: CfgPointId,
    place: &ReferencePlaceId,
    access: LoanAccess<'_>,
) -> Vec<SourceSpan> {
    let place = available_place(place);
    let mut conflicts = Vec::new();
    for (binding, origins) in &state.bindings {
        let mut reachable = false;
        for origin in origins {
            if match access {
                LoanAccess::Direct | LoanAccess::ReadThrough(_) | LoanAccess::Borrow(_) => false,
                LoanAccess::Through(selected) => *origin == selected,
                LoanAccess::WriteThrough(selected) => *origin == selected.origin,
            } {
                continue;
            }
            let loan = match &facts[origin.0] {
                ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                _ => continue,
            };
            let conflicts_with_access = match access {
                LoanAccess::ReadThrough(selected) => {
                    loan.mode == ScalarReferenceMutability::Mutable
                        && loan_has_ancestor(loan, selected)
                        && reference_places_overlap(&available_place(&loan.origin_place), &place)
                }
                LoanAccess::WriteThrough(selected) => {
                    loan_borrows_place(loan, &place) && !loan_has_ancestor(&selected.loan, loan)
                }
                LoanAccess::Borrow(new) => {
                    (new.mode == ScalarReferenceMutability::Mutable
                        || loan.mode == ScalarReferenceMutability::Mutable)
                        && reference_places_overlap(&available_place(&loan.origin_place), &place)
                        && !loan_has_ancestor(new, loan)
                }
                LoanAccess::Direct | LoanAccess::Through(_) => loan_borrows_place(loan, &place),
            };
            if !conflicts_with_access {
                continue;
            }
            if !reachable {
                if !reference_read_reachable(points, point, binding) {
                    break;
                }
                reachable = true;
            }
            conflicts.push(SourceSpan::new(
                loan.origin_place.source.clone(),
                loan.creation_span,
            ));
        }
    }
    conflicts
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
                            && !loan_has_ancestor(&candidate.loan, loan)
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

struct PassiveCheckedReader {
    binding: ReferenceBindingId,
    bind: CfgPointId,
    dereference: CfgPointId,
}

struct PassiveDescriptorRead {
    checked_reader: Option<PassiveCheckedReader>,
    projection: CfgPointId,
    scalar_projection: Option<CfgPointId>,
    literal: CfgPointId,
    call: CfgPointId,
    field: ScalarStructFieldId,
}

fn passive_descriptor_read(
    cfg: &ScalarReferenceCfg,
    marker: &ScalarDescriptorInstall,
    facts: &aggregate::AggregateFlowFacts,
    sites: &FunctionCallSites,
    proofs: &CallablePointFacts,
    structs: &[ScalarStruct],
) -> Option<PassiveDescriptorRead> {
    if sites.call_occurrences.len() != 2 || !sites.allocation_length_call_targets.is_empty()
        || facts.literals.len() != 1
    {
        return None;
    }
    let (&literal, members) = facts.literals.first_key_value()?;
    let [(member, alias_read)] = members.as_slice() else { return None };
    let field = sites.structure.literal_fields.get(member)?;
    if field.structure != marker.structure { return None }
    let CfgPointKind::Read { binding: alias } = &cfg.points.get(alias_read.0)?.kind else { return None };
    let binding_for_literal = |point: CfgPointId| match &cfg.points.get(point.0)?.kind {
        CfgPointKind::Bind { binding } => Some(binding),
        _ => None,
    };
    let mut carrier = None;
    let mut alias_start = None;
    for (&bind, &(rhs, _)) in &facts.installations {
        let binding = binding_for_literal(bind)?;
        if rhs == literal {
            if carrier.replace((binding, bind)).is_some() { return None }
        }
        if binding == alias {
            if !facts.checked_raw_null_initializers.contains(&rhs) || alias_start.replace(bind).is_some() { return None }
        }
    }
    let (carrier, carrier_start) = carrier?;
    let alias_start = alias_start?;
    if alias == carrier || carrier == &marker.binding || alias == &marker.binding
        || cfg.points[alias_start.0].scope != ReferenceScopeId(0)
        || cfg.points[carrier_start.0].scope != ReferenceScopeId(0)
    {
        return None;
    }
    let mut assignment = None;
    let mut call = None;
    let mut carrier_read = None;
    let mut carrier_move = None;
    let mut scalar_projection = None;
    for point in &cfg.points {
        if point.successors.iter().any(|next| next.0 <= point.id.0) { return None }
        match &point.kind {
            CfgPointKind::Assign { target, rhs_point, .. } if &target.binding == alias => {
                if !target.projections.is_empty() || assignment.replace((point.id, *rhs_point)).is_some() { return None }
            }
            CfgPointKind::Assign { target, .. } if &target.binding == carrier => return None,
            CfgPointKind::Read { binding } if binding == alias && point.id != *alias_read => return None,
            CfgPointKind::Read { binding } if binding == carrier => {
                if carrier_read.replace(point.id).is_some() { return None }
            }
            CfgPointKind::Move { place, owner, .. } if owner == carrier => {
                if &place.binding != carrier || !place.projections.is_empty()
                    || carrier_move.replace(point.id).is_some() { return None }
            }
            CfgPointKind::Borrow { place, .. } | CfgPointKind::DeferredMove { place, .. }
                if &place.binding == carrier || &place.binding == alias => return None,
            CfgPointKind::ReadPlace {
                place: ScalarPlace::Field { field: ScalarFieldReference::Resolved(scalar), .. },
                pointer_point: Some(pointer),
                reads_origin: false,
                ..
            } if scalar != field && scalar.structure == marker.structure
                && marker.frozen.install.0 < point.id.0
                && matches!(&cfg.points.get(pointer.0)?.kind,
                    CfgPointKind::Read { binding } if binding == &marker.binding) =>
            {
                let declared = structs.iter().find(|decl| decl.id == scalar.structure)?
                    .fields.iter().find(|decl| decl.id == *scalar)?;
                if !aggregate::ordinary_value(&declared.ty)
                    || scalar_projection.replace(point.id).is_some()
                {
                    return None;
                }
            }
            CfgPointKind::Call { output: 0, .. } if point.id == marker.frozen.rhs => {}
            CfgPointKind::Call { output: 0, target: ResolvedCallTarget::ModuleCallable { .. }, argument_points, .. } => {
                let [actual] = argument_points.as_slice() else { return None };
                if call.replace((point.id, *actual)).is_some() { return None }
            }
            CfgPointKind::Call { expression, target, .. } => {
                if !matches!(&cfg.points[marker.frozen.rhs.0].kind,
                    CfgPointKind::Call { expression: producer, target: source, .. }
                    if expression == producer && target == source)
                {
                    return None;
                }
            }
            _ => {}
        }
    }
    let (assignment, projection) = assignment?;
    let (call, operand) = call?;
    let carrier_read = carrier_read?;
    if operand != carrier_read || !(alias_start.0 < projection.0 && marker.frozen.install.0 < projection.0
        && projection.0 < assignment.0 && assignment.0 < alias_read.0 && alias_read.0 < literal.0
        && literal.0 < carrier_start.0 && carrier_start.0 < carrier_read.0 && carrier_read.0 < call.0)
        || carrier_move.is_some_and(|point| !(carrier_read.0 < point.0 && point.0 < call.0))
        || !facts.projected_reads.get(&projection).copied().unwrap_or(false)
    {
        return None;
    }
    let CfgPointKind::ReadPlace { place: ScalarPlace::Field { field: ScalarFieldReference::Resolved(actual), .. },
        pointer_point: Some(pointer), reads_origin: false, .. } = &cfg.points.get(projection.0)?.kind else { return None };
    if actual != field || !matches!(&cfg.points.get(pointer.0)?.kind,
        CfgPointKind::Read { binding } if binding == &marker.binding)
    {
        return None;
    }
    let reader_site = sites.call_occurrences.iter().find(|site|
        cfg.primary_call_points.get(&site.ordinal) == Some(&call))?;
    let signature = sites.checked_call_signatures.get(&reader_site.ordinal)?;
    let checked_reader = match signature.outputs.outputs.as_slice() {
        outputs if outputs.iter().all(|output| aggregate::ordinary_value(&output.ty) || output.ty == ScalarType::Unit) => None,
        [output @ ScalarOutput { ty: ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Shared, inner,
        }, .. }] if aggregate::ordinary_value(inner) => Some(passive_checked_reader(
            cfg, call, &output.ty, facts, proofs, &sites.structure)?),
        _ => return None,
    };
    Some(PassiveDescriptorRead { checked_reader, projection, scalar_projection, literal, call, field: field.clone() })
}

fn passive_checked_reader(
    cfg: &ScalarReferenceCfg,
    call: CfgPointId,
    ty: &ScalarType,
    facts: &aggregate::AggregateFlowFacts,
    proofs: &CallablePointFacts,
    structure: &OccurrenceStructure,
) -> Option<PassiveCheckedReader> {
    let mut bindings = proofs.definitions.iter().filter_map(|(&position, definition)| match definition {
        BindingDefinition::Initialized { bind, value } if *value == call => Some((position, *bind)),
        _ => None,
    });
    let ((block, item, receiver), bind) = bindings.next()?;
    let point = cfg.points.get(bind.0)?;
    let CfgPointKind::Bind { binding } = &point.kind else { return None };
    if bindings.next().is_some() || point.scope != ReferenceScopeId(0) || block != structure.root {
        return None;
    }
    let BlockItemKind::Binding(declaration) = &structure.blocks.get(block.0)?.items.get(item)?.kind else {
        return None;
    };
    let receiver = declaration.receivers().get(receiver)?;
    if receiver.span != binding.declaration_span || &receiver.ty != ty {
        return None;
    }

    let mut dereference = None;
    for point in &cfg.points[bind.0 + 1..] {
        match &point.kind {
            CfgPointKind::ReadPlace { place: ScalarPlace::Dereference { .. }, pointer_point: Some(pointer),
                reads_origin: false, .. }
                if matches!(&cfg.points.get(pointer.0)?.kind, CfgPointKind::Read { binding: actual } if actual == binding) =>
            {
                if dereference.replace((point, *pointer)).is_some() { return None }
            }
            CfgPointKind::Assign { target, .. } | CfgPointKind::Borrow { place: target, .. }
            | CfgPointKind::Move { place: target, .. } | CfgPointKind::DeferredMove { place: target, .. }
                if &target.binding == binding => return None,
            _ => {}
        }
    }
    let (dereference, pointer) = dereference?;
    let mut guarded = false;
    for point in &cfg.points[bind.0 + 1..dereference.id.0] {
        let CfgPointKind::Branch { .. } = &point.kind else { continue };
        let compare = &cfg.points[point.id.0 - 1];
        let Some(&(read, true)) = facts.passive_checked_comparisons.get(&compare.id) else { continue };
        if !compare.successors.contains(&point.id)
            || !matches!(&cfg.points[read.0].kind, CfgPointKind::Read { binding: actual } if actual == binding)
        {
            continue;
        }
        let Some(then) = point.successors.first().and_then(|then| cfg.points.get(then.0)) else { continue };
        if matches!(&then.kind, CfgPointKind::ScopeEnter) && then.scope == dereference.scope
            && then.id.0 < dereference.id.0
        {
            guarded = true;
            break;
        }
    }
    if !guarded || cfg.points[bind.0 + 1..].iter().any(|point| {
        matches!(&point.kind, CfgPointKind::Read { binding: actual } if actual == binding)
            && point.id != pointer && !facts.passive_checked_comparisons.values().any(|(read, _)| *read == point.id)
    }) {
        return None;
    }
    Some(PassiveCheckedReader { binding: binding.clone(), bind, dereference: dereference.id })
}

struct ScalarDescriptorReplay<'a> {
    candidate: &'a ScalarDescriptorInstall,
    passive: Option<&'a PassiveDescriptorRead>,
    invalid: bool,
    exited: bool,
}

impl ScalarDescriptorReplay<'_> {
    fn observe(
        &mut self,
        point: &ReferenceFlowPoint,
        state: &ReferenceEdgeState,
        facts: &[ReferenceOrigin],
    ) {
        let candidate = self.candidate;
        let owns = |id: &ReferenceOriginId| matches!(&facts[id.0], ReferenceOrigin::Fresh { loan, .. }
            if matches!(loan.origin_place.binding.kind, ReferenceBindingKind::CallOutput { point, output }
                if point == candidate.frozen.rhs && output == candidate.output));
        let carries = |origins: Option<&BTreeSet<ReferenceOriginId>>| {
            origins.is_some_and(|origins| origins.iter().any(owns))
        };
        let competing_owner = || candidate.validate_backing && point.possible_origins.as_ref().is_some_and(|origins| {
            origins.iter().any(|id| matches!(&facts[id.0], ReferenceOrigin::Fresh { .. }))
        });
        let reader = self.passive.and_then(|proof| proof.checked_reader.as_ref().map(|reader| (reader, proof.call)));
        let reader_origins = |origins: Option<&BTreeSet<ReferenceOriginId>>, reader_call: CfgPointId| {
            origins.is_some_and(|origins| !origins.is_empty() && origins.iter().all(|id| match &facts[id.0] {
                ReferenceOrigin::Fresh { loan, .. } => matches!(loan.origin_place.binding.kind,
                    ReferenceBindingKind::CallOutput { point, output: 0 } if point == reader_call),
                ReferenceOrigin::Null { .. } => true,
                _ => false,
            }))
        };
        match &point.kind {
            CfgPointKind::Assign { target, rhs_point, previous_origins, .. } if point.id == candidate.frozen.install => {
                self.invalid |= target.binding != candidate.binding || *rhs_point != candidate.frozen.rhs
                    || previous_origins.is_empty()
                    || !previous_origins.iter().all(|id| matches!(&facts[id.0], ReferenceOrigin::Null { .. }))
                    || !carries(point.possible_origins.as_ref())
                    || point.possible_origins.as_ref().is_some_and(|origins| origins.iter().any(|id| {
                        matches!(&facts[id.0], ReferenceOrigin::Invalid { .. })
                    }));
            }
            CfgPointKind::Assign { target, .. } => {
                self.invalid |= target.binding == candidate.binding || carries(point.possible_origins.as_ref()) || competing_owner();
            }
            CfgPointKind::Bind { binding } if reader.is_some_and(|(reader, _)| reader.bind == point.id) => {
                let (reader, call) = reader.expect("exact passive reader binding");
                self.invalid |= binding != &reader.binding || !reader_origins(point.possible_origins.as_ref(), call);
            }
            CfgPointKind::Bind { .. } => {
                self.invalid |= carries(point.possible_origins.as_ref()) || competing_owner();
            }
            CfgPointKind::ReadPlace { pointer_point, .. }
                if reader.is_some_and(|(reader, _)| reader.dereference == point.id) =>
            {
                let (_, call) = reader.expect("exact passive reader dereference");
                self.invalid |= !reader_origins(pointer_point.and_then(|p| state.point_values.get(p)), call);
            }
            CfgPointKind::ReadPlace { pointer_point, .. } if candidate.validate_backing => {
                self.invalid |= !self.passive.is_some_and(|proof| proof.projection == point.id
                        || proof.scalar_projection == Some(point.id))
                    || !pointer_point.and_then(|pointer| state.point_values.get(pointer)).is_some_and(|origins| {
                        !origins.is_empty() && origins.iter().all(owns)
                    });
            }
            CfgPointKind::Borrow { place, .. }
            | CfgPointKind::Move { place, .. }
            | CfgPointKind::DeferredMove { place, .. } => {
                self.invalid |= place.binding == candidate.binding;
            }
            CfgPointKind::Evaluate(ScalarExpression::StructLiteral { .. })
                if self.passive.is_some_and(|proof| proof.literal == point.id) => {}
            CfgPointKind::Evaluate(ScalarExpression::RawAddress { .. } | ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. })
            | CfgPointKind::AssignThrough { .. } | CfgPointKind::MoveThrough { .. }
            | CfgPointKind::DeferredMoveThrough { .. } | CfgPointKind::Release { .. } | CfgPointKind::Invalidate { .. } => {
                self.invalid = true;
            }
            CfgPointKind::Call { output, argument_points, .. } => {
                self.invalid |= (candidate.validate_backing && *output == 0 && point.id != candidate.frozen.rhs
                    && !self.passive.is_some_and(|proof| proof.call == point.id))
                    || argument_points.iter().any(|&point| carries(state.point_values.get(point)));
            }
            CfgPointKind::ScopeExit if point.scope == ReferenceScopeId(0) => {
                self.exited = true;
                self.invalid |= !carries(state.bindings.get(&candidate.binding));
                if let Some((reader, call)) = reader {
                    self.invalid |= !reader_origins(state.bindings.get(&reader.binding), call);
                }
            }
            _ => {}
        }
    }
}

struct ReferenceTransferCfg<'a> {
    initial_points: &'a [ReferenceFlowPoint],
    pending_named_reborrow: Option<&'a super::control::PendingNamedReborrow>,
    resolve_pending_named_reborrow: bool,
    raw_address_exposures: &'a [(CfgPointId, Option<ReferenceBindingId>)],
    points: &'a mut [ReferenceFlowPoint],
    facts: &'a mut Vec<ReferenceOrigin>,
    allocations: &'a BTreeSet<ReferenceBindingId>,
    checked_join_values: &'a [(CfgPointId, [Option<CfgPointId>; 2])],
    checked_block_values: &'a [(CfgPointId, CfgPointId)],
    checked_binding_initializers: &'a [(CfgPointId, CfgPointId)],
    resolved_checked_call_outputs: &'a [CfgPointId],
    checked_pointee_reads: &'a [CfgPointId],
    deferred_child_scalar_stores: &'a [CfgPointId],
    named_reborrow_parents: &'a [super::control::NamedReborrowParent],
}

impl<'a> ReferenceTransferCfg<'a> {
    fn new(cfg: &'a mut ScalarReferenceCfg, proof: Option<&'a CallablePointFacts>) -> Self {
        Self {
            initial_points: &cfg.initial_points,
            pending_named_reborrow: proof.and_then(|proof| proof.pending_named_reborrow.as_ref()),
            resolve_pending_named_reborrow: false,
            raw_address_exposures: proof.map_or(&[], |proof| &proof.raw_address_exposures),
            points: &mut cfg.points,
            facts: &mut cfg.facts,
            allocations: &cfg.allocations,
            checked_join_values: proof.map_or(&[], |proof| &proof.checked_join_values),
            checked_block_values: proof.map_or(&[], |proof| &proof.checked_block_values),
            checked_binding_initializers: proof.map_or(&[], |proof| &proof.checked_binding_initializers),
            resolved_checked_call_outputs: proof.map_or(&[], |proof| &proof.resolved_checked_call_outputs),
            checked_pointee_reads: proof.map_or(&[], |proof| &proof.checked_pointee_reads),
            deferred_child_scalar_stores: proof.map_or(&[], |proof| &proof.deferred_child_scalar_stores),
            named_reborrow_parents: proof.map_or(&[], |proof| &proof.named_reborrow_parents),
        }
    }
}

fn deferred_named_reborrow_allowed(function: &ScalarFunction, calls: &FunctionCallSites) -> bool {
    function.generic_parameters.is_empty()
        && matches!(&function.signature, ScalarType::Callable { outputs, .. }
            if match outputs.outputs.as_slice() {
                [] => true,
                [ScalarOutput { ty, .. }] => *ty == ScalarType::Unit || aggregate::ordinary_value(ty),
                _ => false,
            })
        && matches!(calls.named_place_roots, NamedPlaceRootFacts::Prepared(_))
        && calls.structure.module_storage_uses.is_empty()
}

fn deferred_reborrow_owner(pending: &super::control::PendingNamedReborrow) -> &ReferenceBindingId {
    match &pending.root {
        super::control::DeferredReborrowRoot::Formal { .. } => &pending.handle,
        super::control::DeferredReborrowRoot::Local { owner, .. } => owner,
    }
}

fn deferred_reborrow_fact<'a>(
    pending: &super::control::PendingNamedReborrow,
    facts: &'a [ReferenceOrigin],
) -> Option<(&'a ReferenceOrigin, &'a ReferenceLoanId)> {
    let fact = facts.get(pending.origin.0)?;
    let loan = match (&pending.root, fact) {
        (super::control::DeferredReborrowRoot::Formal { parameter: expected },
            ReferenceOrigin::BorrowedFrom { parameter, loan }) if expected == parameter => loan,
        (super::control::DeferredReborrowRoot::Local { .. }, ReferenceOrigin::Fresh { loan, .. }) => loan,
        _ => return None,
    };
    (loan.parent.is_none() && loan.mode == ScalarReferenceMutability::Mutable
        && loan.origin_place.binding == *deferred_reborrow_owner(pending)).then_some((fact, loan))
}

fn deferred_reborrow_child(origin: &ReferenceOrigin, loan: ReferenceLoanId) -> ReferenceOrigin {
    match origin {
        ReferenceOrigin::BorrowedFrom { parameter, .. } => ReferenceOrigin::BorrowedFrom { parameter: *parameter, loan },
        ReferenceOrigin::Fresh { allocation, .. } => ReferenceOrigin::Fresh { allocation: allocation.clone(), loan },
        _ => unreachable!("validated deferred reborrow root"),
    }
}

fn is_deferred_reborrow_child(parent: &ReferenceOrigin, child: &ReferenceOrigin, loan: &ReferenceLoanId) -> bool {
    match (parent, child) {
        (ReferenceOrigin::BorrowedFrom { parameter: root, .. },
            ReferenceOrigin::BorrowedFrom { parameter, loan: observed }) => root == parameter && loan == observed,
        (ReferenceOrigin::Fresh { allocation: root, .. },
            ReferenceOrigin::Fresh { allocation, loan: observed }) => root == allocation && loan == observed,
        _ => false,
    }
}

fn deferred_local_owner_store(
    pending: &super::control::PendingNamedReborrow,
    point: &ReferenceFlowPoint,
    points: &[ReferenceFlowPoint],
) -> bool {
    let super::control::DeferredReborrowRoot::Local { owner, borrow } = &pending.root else {
        return false;
    };
    matches!(&point.kind, CfgPointKind::Assign { target, .. }
        if &target.binding == owner && target.projections.is_empty()
            && matches!(target.place, ScalarPlace::Name { .. }))
        && point.scope == points[borrow.0].scope
        && pending.sites.last().is_some_and(|site| site.point < point.id)
}

fn contains_loan_site(binding: &ReferenceBindingId, point: &ReferenceFlowPoint) -> bool {
    let site = point.source_span.range;
    binding.block_span.start <= site.start && site.end <= binding.block_span.end
}

fn is_pending_shared_reborrow(point: &ReferenceFlowPoint) -> bool {
    match &point.kind {
        CfgPointKind::Evaluate(ScalarExpression::CheckedAddress { mutability, .. }) =>
            *mutability == ScalarReferenceMutability::Shared,
        CfgPointKind::Borrow { loan, .. } => loan.mode == ScalarReferenceMutability::Shared,
        _ => false,
    }
}

fn exact_live_reborrow_holder(
    state: &ReferenceEdgeState,
    binding: &ReferenceBindingId,
    origin: ReferenceOriginId,
) -> bool {
    state.bindings.get(binding).is_some_and(|origins| origins.len() == 1 && origins.contains(&origin))
        && !state.released.contains(binding)
        && !state.invalidated.contains(binding)
        && !state.uninitialized.contains(binding)
        && unavailable_checked_handles(state, binding, &[], CheckedHandleStorage::Lexical).is_empty()
}

fn transfer_reference_facts(
    cfg: ReferenceTransferCfg<'_>,
    deferred_policies: &BTreeMap<usize, CopyPolicy>,
    contracts: &HashMap<CfgPointId, &[ReferenceOrigin]>,
    mut aggregate: Option<(
        &aggregate::AggregateFlowFacts,
        &FunctionCallSites,
        &mut aggregate::AggregateFlowResult,
    )>,
    mut descriptor: Option<&mut ScalarDescriptorReplay<'_>>,
) -> Vec<ReferenceAnalysisError> {
    let original_points = cfg.initial_points;
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
    let mut strictly_forward = true;
    let mut aggregate_expiry_allowed = aggregate.is_some();
    let pending = cfg.pending_named_reborrow;
    let mut local_definition = None;
    let mut child_holders = HashMap::new();
    let mut pending_allowed = pending.is_some_and(|pending| {
        let has_shared = pending.sites.iter().any(|site|
            is_pending_shared_reborrow(&original_points[site.point.0]));
        if pending.sites.len() > 1 || has_shared {
            for site in &pending.sites {
                let Some(holder) = site.holder else { return false };
                let point = &original_points[holder.0];
                if holder <= site.point || point.scope != original_points[site.point.0].scope {
                    return false;
                }
                let CfgPointKind::Bind { binding } = &point.kind else { return false };
                if child_holders.insert(binding, site.point).is_some() {
                    return false;
                }
            }
        }
        if has_shared {
            for &(bind, value) in cfg.checked_binding_initializers {
                let target = &original_points[bind.0];
                let read = &original_points[value.0];
                let (CfgPointKind::Bind { binding: destination }, CfgPointKind::Read { binding: source }) =
                    (&target.kind, &read.kind) else { continue };
                if value >= bind || read.scope != target.scope || read.successors.as_slice() != [bind]
                    || child_holders.contains_key(destination)
                {
                    continue;
                }
                let Some(producer) = child_holders.get(source).copied() else { continue };
                let Ok(index) = pending.sites.binary_search_by_key(&producer, |site| site.point) else { continue };
                let Some(holder) = pending.sites[index].holder else { continue };
                let holder = &original_points[holder.0];
                if holder.id < value && holder.scope == read.scope
                    && matches!(&holder.kind, CfgPointKind::Bind { binding } if binding == source)
                    && is_pending_shared_reborrow(&original_points[producer.0])
                {
                    child_holders.insert(destination, producer);
                }
            }
        }
        !cfg.raw_address_exposures.iter().any(|(_, root)| root.as_ref().is_none_or(|root|
            *root == pending.handle || root == deferred_reborrow_owner(pending) || child_holders.contains_key(root)))
    });
    let exact_child_store = |write: &ReferenceFlowPoint| {
        let CfgPointKind::AssignThrough {
            place: ScalarPlace::Dereference { pointer, .. }, pointer_point, rhs_point, ..
        } = &write.kind else { return None };
        if child_holders.is_empty() || !matches!(pointer.as_ref(), ScalarExpression::Name { .. })
            || cfg.deferred_child_scalar_stores.binary_search(&write.id).is_err()
            || grouped_installs.contains(&write.id)
            || !write.possible_origins.as_ref().is_some_and(BTreeSet::is_empty)
            || rhs_point >= pointer_point || pointer_point.0 + 1 != write.id.0
        {
            return None;
        }
        let read = &original_points[pointer_point.0];
        let CfgPointKind::Read { binding } = &read.kind else { return None };
        if read.successors.as_slice() != [write.id] || read.scope != write.scope {
            return None;
        }
        child_holders.get(binding).copied()
    };
    for (index, point) in original_points.iter().enumerate() {
        strictly_forward = strictly_forward && point.id.0 == index
            && point.successors.iter().all(|successor| successor.0 > index && successor.0 < original_points.len());
        aggregate_expiry_allowed = aggregate_expiry_allowed && strictly_forward
            && !matches!(&point.kind, CfgPointKind::MoveThrough { place, .. }
                if !matches!(place, ScalarPlace::Dereference { .. }));
        if let Some(pending) = pending.filter(|_| pending_allowed) {
            if let super::control::DeferredReborrowRoot::Local { owner, borrow } = &pending.root {
                if matches!(&point.kind, CfgPointKind::Bind { binding } if binding == owner) {
                    if point.id < *borrow && point.scope == original_points[borrow.0].scope
                        && !cfg.allocations.contains(owner) && local_definition.is_none()
                    {
                        local_definition = Some(point.id);
                    } else {
                        pending_allowed = false;
                    }
                }
            }
            pending_allowed &= !match &point.kind {
                CfgPointKind::Call { .. } | CfgPointKind::Invalidate { .. } | CfgPointKind::Release { .. } => true,
                CfgPointKind::Assign { target, .. } => target.binding == pending.handle
                    || (&target.binding == deferred_reborrow_owner(pending)
                        && (grouped_installs.contains(&point.id) || !deferred_local_owner_store(pending, point, original_points)))
                    || child_holders.contains_key(&target.binding),
                CfgPointKind::AssignThrough { .. } => exact_child_store(point).is_none()
                    && pending.sites.last().is_some_and(|last| point.id < last.point),
                CfgPointKind::MoveThrough { .. }
                | CfgPointKind::DeferredMoveThrough { .. } => pending.sites.last()
                    .is_some_and(|last| point.id < last.point),
                CfgPointKind::Move { place, .. } | CfgPointKind::DeferredMove { place, .. } =>
                    child_holders.contains_key(&place.binding),
                CfgPointKind::Read { binding } if child_holders.contains_key(binding) =>
                    original_points.get(index + 1).is_none_or(|next| {
                        if exact_child_store(next).is_some() {
                            return false;
                        }
                        if matches!(&next.kind, CfgPointKind::Bind { binding: destination }
                            if child_holders.get(destination) == child_holders.get(binding))
                            && cfg.checked_binding_initializers.binary_search_by_key(&next.id, |&(bind, _)| bind)
                                .ok().is_some_and(|index| cfg.checked_binding_initializers[index].1 == point.id)
                        {
                            return false;
                        }
                        if point.successors.as_slice() == [next.id]
                            && pending.sites.binary_search_by_key(&next.id, |site| site.point).ok()
                                .is_some_and(|position| {
                                    let site = &pending.sites[position];
                                    site.read == point.id && site.parent.is_some_and(|(parent, _)|
                                        child_holders.get(binding).copied() == Some(parent))
                                })
                        {
                            return false;
                        }
                        point.successors.as_slice() != [next.id]
                            || !matches!(&next.kind, CfgPointKind::ReadPlace {
                                place: ScalarPlace::Dereference { .. },
                                pointer_point: Some(pointer), reads_origin: false, ..
                            } if *pointer == point.id)
                            || cfg.checked_pointee_reads.binary_search(&next.id).is_err()
                    }),
                CfgPointKind::Borrow { place, loan } => loan.parent.is_none()
                    && (place.binding == pending.handle || child_holders.contains_key(&place.binding)
                        || (&place.binding == deferred_reborrow_owner(pending)
                            && !matches!(pending.root, super::control::DeferredReborrowRoot::Local { borrow, .. } if borrow == point.id))),
                _ => false,
            };
        }
        let dependencies = match &point.kind {
            CfgPointKind::Invalidate { address_point, .. }
            | CfgPointKind::Release {
                target: ReleaseTarget::Checked { address_point, .. },
            } => vec![*address_point],
            CfgPointKind::MoveThrough { pointer_point, .. }
            | CfgPointKind::DeferredMoveThrough { pointer_point, .. } => vec![*pointer_point],
            CfgPointKind::Assign { rhs_point, .. } => vec![*rhs_point],
            CfgPointKind::ReturnOutput { value_point, .. } => vec![*value_point],
            CfgPointKind::ConditionalOutput {
                then_point,
                else_point,
            } => vec![*then_point, *else_point],
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
    if !strictly_forward {
        if let Some(descriptor) = descriptor.as_deref_mut() {
            descriptor.invalid = true;
        }
        descriptor = None;
    }
    if let Some(pending) = pending {
        for site in &pending.sites {
            last_use[site.read.0] = last_use[site.read.0].max(site.point.0);
        }
    }
    for &(join, terminals) in cfg.checked_join_values {
        for value in terminals.into_iter().flatten() {
            last_use[value.0] = last_use[value.0].max(join.0);
        }
    }
    for &(wrapper, value) in cfg.checked_block_values {
        last_use[value.0] = last_use[value.0].max(wrapper.0);
    }
    for &(bind, value) in cfg.checked_binding_initializers {
        last_use[value.0] = last_use[value.0].max(bind.0);
    }
    for parent in cfg.named_reborrow_parents {
        last_use[parent.pointer.0] = last_use[parent.pointer.0].max(parent.borrow.0);
    }
    let aggregate_deadlines = aggregate.as_ref()
        .filter(|_| aggregate_expiry_allowed)
        .map(|(facts, _, _)| facts.value_deadlines(&last_use));
    let mut observed = vec![None::<ReferenceFlowPoint>; cfg.points.len()];
    let mut incoming = vec![Vec::<ReferenceEdgeState>::new(); cfg.points.len()];
    incoming[0].push(ReferenceEdgeState {
        aggregate: aggregate.as_ref().map(|(facts, _, _)| facts.entry_state()),
        ..ReferenceEdgeState::default()
    });
    let mut fallback = (!strictly_forward).then(|| {
        let mut queued = vec![false; cfg.points.len()];
        queued[0] = true;
        (
            vec![Vec::<ReferenceEdgeState>::new(); cfg.points.len()],
            queued,
            std::collections::VecDeque::from([CfgPointId(0)]),
        )
    });
    let mut forward_cursor = 0;
    let mut forward_single_observation = strictly_forward.then(|| vec![false; cfg.points.len()]);
    let mut errors = Vec::new();
    let mut pending_sites = pending.map_or(&[][..], |pending| pending.sites.as_slice()).iter().peekable();
    loop {
        let id = if let Some((_, queued, ready)) = &mut fallback {
            let Some(id) = ready.pop_front() else { break };
            queued[id.0] = false;
            id
        } else {
            let Some(offset) = incoming[forward_cursor..].iter().position(|slot| !slot.is_empty()) else {
                break;
            };
            let id = CfgPointId(forward_cursor + offset);
            forward_cursor = id.0 + 1;
            id
        };
        let common_aggregate_cutoff = aggregate_deadlines.as_ref()
            .and_then(|_| original_points[id.0].successors.iter().copied().min());
        let states = std::mem::take(&mut incoming[id.0]);
        if let Some(single_observation) = &mut forward_single_observation {
            if matches!(&original_points[id.0].kind, CfgPointKind::MoveThrough { place, .. }
                if !matches!(place, ScalarPlace::Dereference { .. }))
            {
                single_observation[id.0] = states.len() == 1;
            }
        }
        let pending_here = pending.and_then(|pending| {
            let site = if strictly_forward {
                while pending_sites.peek().is_some_and(|site| site.point < id) {
                    pending_sites.next();
                }
                if pending_sites.peek().is_some_and(|site| site.point == id) {
                    pending_sites.next()
                } else {
                    None
                }
            } else {
                pending.sites.binary_search_by_key(&id, |site| site.point).ok()
                    .map(|index| &pending.sites[index])
            };
            site.map(|site| (pending, site))
        });
        let selected_parent = pending_here.and_then(|(pending, site)| {
            let (root, root_loan) = deferred_reborrow_fact(pending, cfg.facts)?;
            let Some((producer, holder)) = site.parent else {
                return Some((pending.origin, &pending.handle));
            };
            let read = &original_points[site.read.0];
            let declaration = &original_points[holder.0];
            let borrow = &original_points[site.point.0];
            let CfgPointKind::Bind { binding } = &declaration.kind else { return None };
            if producer >= holder || holder >= site.read
                || read.successors.as_slice() != [site.point]
                || read.scope != borrow.scope
                || !contains_loan_site(binding, borrow)
                || !matches!(&read.kind, CfgPointKind::Read { binding: read_binding } if read_binding == binding)
                || child_holders.get(binding).copied() != Some(producer)
            {
                return None;
            }
            let source = &cfg.points[producer.0];
            let CfgPointKind::Borrow { loan, .. } = &source.kind else { return None };
            let origins = source.possible_origins.as_ref()?;
            let origin = *origins.first().filter(|_| origins.len() == 1)?;
            if source.scope != declaration.scope || loan.parent.as_deref() != Some(root_loan)
                || loan.mode != ScalarReferenceMutability::Mutable
                || !is_deferred_reborrow_child(root, cfg.facts.get(origin.0)?, loan)
            {
                return None;
            }
            Some((origin, binding))
        });
        let effective_point = pending_here.map(|(pending, site)| {
            let mut point = original_points[id.0].clone();
            let valid = pending_allowed && strictly_forward
                && matches!(&original_points[pending.entry.0].kind,
                    CfgPointKind::Bind { binding } if *binding == pending.handle)
                && original_points[pending.entry.0].possible_origins.as_ref()
                    .is_some_and(|origins| origins.len() == 1 && origins.contains(&pending.origin))
                && match &pending.root {
                    super::control::DeferredReborrowRoot::Formal { .. } => true,
                    super::control::DeferredReborrowRoot::Local { owner, borrow } => {
                        let initializer = &original_points[borrow.0];
                        local_definition.is_some()
                            && *borrow < pending.entry
                            && initializer.scope == original_points[pending.entry.0].scope
                            && (site.parent.is_some()
                                || initializer.scope == point.scope
                                || (is_pending_shared_reborrow(&point) && contains_loan_site(&pending.handle, &point)))
                            && matches!(&initializer.kind, CfgPointKind::Borrow { place, loan }
                                if place.binding == *owner && loan.parent.is_none())
                            && initializer.possible_origins.as_ref()
                                .is_some_and(|origins| origins.len() == 1 && origins.contains(&pending.origin))
                    }
                }
                && !states.is_empty()
                && selected_parent.is_some_and(|(origin, binding)| states.iter().all(|state| {
                    let owner = deferred_reborrow_owner(pending);
                    let read = state.point_values.get(site.read);
                    exact_live_reborrow_holder(state, &pending.handle, pending.origin)
                        && (binding == &pending.handle || exact_live_reborrow_holder(state, binding, origin))
                        && read.is_some_and(|origins| origins.len() == 1 && origins.contains(&origin))
                        && !state.released.contains(owner) && !state.invalidated.contains(owner)
                        && !state.uninitialized.contains(owner)
                        && deferred_reborrow_fact(pending, cfg.facts)
                            .is_some_and(|(_, loan)| !unavailable_place(state, &loan.origin_place))
                }));
            if valid {
                let (origin, _) = selected_parent.expect("validated immediate reborrow parent");
                let parent_fact = &cfg.facts[origin.0];
                let parent = match parent_fact {
                    ReferenceOrigin::BorrowedFrom { loan, .. } | ReferenceOrigin::Fresh { loan, .. } => loan,
                    _ => unreachable!("validated immediate reborrow loan"),
                };
                if cfg.resolve_pending_named_reborrow {
                    let CfgPointKind::Evaluate(ScalarExpression::CheckedAddress { place, mutability, .. }) = &point.kind else {
                        unreachable!("provisional named reborrow expression")
                    };
                    let mut origin_place = parent.origin_place.clone();
                    origin_place.place = place.clone();
                    origin_place.projections.clear();
                    collect_reference_projections(place, Some(parent), &mut origin_place.projections);
                    let loan = ReferenceLoanId {
                        mode: *mutability,
                        origin_place,
                        creation_span: point.source_span.range,
                        parent: Some(Box::new(parent.clone())),
                    };
                    let child = deferred_reborrow_child(parent_fact, loan.clone());
                    let child_id = match cfg.facts.iter().position(|origin| *origin == child) {
                        Some(index) => ReferenceOriginId(index),
                        None => {
                            let id = ReferenceOriginId(cfg.facts.len());
                            cfg.facts.push(child);
                            id
                        }
                    };
                    point.kind = CfgPointKind::Borrow { place: loan.origin_place.clone(), loan };
                    point.possible_origins = Some(BTreeSet::from([child_id]));
                    return point;
                }
                if matches!(&point.kind, CfgPointKind::Borrow { loan, .. }
                    if loan.parent.as_deref() == Some(parent)
                        && point.possible_origins.as_ref().is_some_and(|origins| {
                            origins.len() == 1 && origins.first().is_some_and(|child|
                                cfg.facts.get(child.0).is_some_and(|child| is_deferred_reborrow_child(parent_fact, child, loan)))
                        }))
                {
                    return point;
                }
            }
            errors.push(ReferenceAnalysisError::MissingOrigin {
                source_span: point.source_span.clone(),
                binding_id: pending.handle.clone(),
                reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
            });
            point.possible_origins = Some(BTreeSet::new());
            point
        });
        let initial_point = effective_point.as_ref().unwrap_or(&original_points[id.0]);
        if effective_point.is_some() {
            cfg.points[id.0] = initial_point.clone();
        }
        let mut ran_point = false;
        for mut state in states {
            let reset_point = if let Some((visited, _, _)) = &mut fallback {
                if visited[id.0].contains(&state) {
                    continue;
                }
                let already_visited = !visited[id.0].is_empty();
                visited[id.0].push(state.clone());
                already_visited
            } else {
                std::mem::replace(&mut ran_point, true)
            };
            if let (Some((facts, sites, result)), Some(flow)) =
                (&mut aggregate, &mut state.aggregate)
            {
                facts.visit(initial_point, flow, result, sites, aggregate_deadlines.as_deref());
            }
            let mut move_alternatives = Vec::new();
            if reset_point {
                cfg.points[id.0] = initial_point.clone();
            }
            let invalidation_address = match &cfg.points[id.0].kind {
                CfgPointKind::Invalidate {
                    address_point,
                    address_span,
                    ..
                } => Some((
                    address_span.clone(),
                    state.point_values.get(*address_point).cloned(),
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
                    state.point_values.get(*address_point).cloned(),
                )),
                _ => None,
            };
            let move_pointer = match &cfg.points[id.0].kind {
                CfgPointKind::MoveThrough { pointer_point, .. }
                | CfgPointKind::DeferredMoveThrough { pointer_point, .. } => {
                    Some(state.point_values.get(*pointer_point).cloned())
                }
                _ => None,
            };
            let assignment_rhs = match &cfg.points[id.0].kind {
                CfgPointKind::Assign { rhs_point, .. }
                | CfgPointKind::AssignThrough { rhs_point, .. } => {
                    Some(state.point_values.get(*rhs_point).cloned())
                }
                _ => None,
            };
            let assignment_pointer = match &cfg.points[id.0].kind {
                CfgPointKind::AssignThrough { pointer_point, .. } => {
                    let origins = state.point_values.get(*pointer_point);
                    let valid = exact_child_store(&original_points[id.0]).is_none_or(|child| {
                        origins.is_some_and(|origins| origins.len() == 1
                            && matches!(&cfg.points[child.0].kind, CfgPointKind::Borrow { .. })
                            && cfg.points[child.0].possible_origins.as_ref() == Some(origins))
                    });
                    Some(origins.filter(|_| valid).cloned())
                }
                _ => None,
            };
            let place_read_pointer = match &cfg.points[id.0].kind {
                CfgPointKind::ReadPlace { pointer_point, .. } => {
                    pointer_point
                        .map(|pointer_point| state.point_values.get(pointer_point).cloned())
                }
                _ => None,
            };
            let call_origins = if let CfgPointKind::Call { output, argument_points, .. } =
                &cfg.points[id.0].kind
            {
                contracts.get(&id).map(|&origins| {
                    let mut results = BTreeSet::new();
                    for origin in origins {
                        let actual = match origin {
                            ReferenceOrigin::BorrowedFrom { parameter, .. } => {
                                if let Some(argument) = state.point_values.get(argument_points[*parameter]) {
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
            let isolated_call_output = matches!(point.kind, CfgPointKind::Call { .. })
                && cfg.resolved_checked_call_outputs.binary_search(&id).is_ok();
            if isolated_call_output && point.possible_origins.as_ref().is_some_and(BTreeSet::is_empty) {
                point.possible_origins = None;
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
                        let checked_pointee = cfg.checked_pointee_reads.binary_search(&id).is_ok();
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
                                    if checked_pointee {
                                        let origins = conflicting_live_loan(
                                            original_points,
                                            &cfg.facts,
                                            &state,
                                            id,
                                            &resolved,
                                            LoanAccess::ReadThrough(loan),
                                        );
                                        if !origins.is_empty() {
                                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                                source_span: point.source_span.clone(),
                                                origins,
                                            });
                                        }
                                    }
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
                        let place = match &point.kind {
                            CfgPointKind::MoveThrough { place, .. }
                            | CfgPointKind::DeferredMoveThrough { place, .. } => place.clone(),
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
                        point.possible_origins = Some(origins);
                        if let CfgPointKind::MoveThrough {
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
                        original_points,
                        &cfg.facts,
                        &state,
                        id,
                        &candidate.place,
                        LoanAccess::Through(candidate.origin),
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
                            let candidates = match state.point_values.get(*pointer_point) {
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
            let (read_places, handle_storage) = match &point.kind {
                CfgPointKind::ReadPlace { candidates, pointer_point, .. } => {
                    let storage = if state.moved_checked_handles.is_empty() {
                        None
                    } else if pointer_point.is_none() {
                        Some(CheckedHandleStorage::Lexical)
                    } else if cfg.checked_pointee_reads.binary_search(&id).is_ok() {
                        Some(CheckedHandleStorage::Pointee)
                    } else {
                        None
                    };
                    (candidates.as_slice(), storage)
                }
                _ => (&[][..], None),
            };
            for place in read_places {
                if let Some(storage) = handle_storage {
                    let normalized;
                    let storage_place = if matches!(storage, CheckedHandleStorage::Pointee) {
                        normalized = available_place(place);
                        &normalized
                    } else {
                        place
                    };
                    if is_lexical_field_path(&storage_place.projections) {
                        let origins = unavailable_checked_handles(
                            &state, &storage_place.binding, &storage_place.projections, storage,
                        );
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
                    }
                }
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
                        ScalarType::CheckedReference {
                            mutability: ScalarReferenceMutability::Mutable,
                            ..
                        },
                    ..
                } if is_lexical_field_path(&place.projections) => {
                    let moved = state
                        .moved_checked_handles
                        .entry(place.binding.clone())
                        .or_default();
                    if !moved.iter().any(|(path, _)| *path == place.projections) {
                        moved.push((place.projections.clone(), point.source_span.clone()));
                    }
                }
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
                        original_points,
                        &cfg.facts,
                        &state,
                        id,
                        &place,
                        LoanAccess::Direct,
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
                CfgPointKind::MoveThrough { place: through, candidates, .. }
                | CfgPointKind::DeferredMoveThrough { place: through, candidates, .. }
                    if matches!(point.kind, CfgPointKind::MoveThrough { .. })
                        || deferred_policies.get(&id.0) == Some(&CopyPolicy::Move) =>
                {
                    let singleton = candidates.len() == 1;
                    for candidate in candidates {
                        let place = available_place(&candidate.place);
                        let mut origins = unavailable_origins(&state, &place);
                        origins.extend(conflicting_live_loan(
                            original_points,
                            &cfg.facts,
                            &state,
                            id,
                            &place,
                            LoanAccess::Through(candidate.origin),
                        ));
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
                        let move_origin = || {
                            if matches!(through, ScalarPlace::Dereference { .. }) {
                                SourceSpan::new(candidate.place.source.clone(), candidate.loan.creation_span)
                            } else {
                                point.source_span.clone()
                            }
                        };
                        if singleton {
                            if !state.moved.iter().any(|(moved, _)| moved == &place) {
                                state.moved.push((place, move_origin()));
                            }
                        } else if !move_alternatives.iter().any(|(moved, _)| moved == &place) {
                            move_alternatives.push((place, move_origin()));
                        }
                    }
                }
                CfgPointKind::Assign { target, .. } => {
                    let target = available_place(target);
                    if !target.projections.is_empty() || (pending_allowed
                        && pending.is_some_and(|pending| deferred_local_owner_store(pending, point, original_points)))
                    {
                        let origins = conflicting_live_loan(
                            original_points,
                            &cfg.facts,
                            &state,
                            id,
                            &target,
                            LoanAccess::Direct,
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
                    if !state.moved_checked_handles.is_empty() {
                        if let Some((first, alternatives)) = candidates.split_first() {
                            let target = available_place(&first.place);
                            if alternatives.iter().all(|candidate| {
                                reference_places_same(&target, &available_place(&candidate.place))
                            }) {
                                reinitialize_checked_handles(&mut state, &target, CheckedHandleStorage::Pointee);
                            }
                        }
                    }
                    for candidate in candidates {
                        let target = available_place(&candidate.place);
                        let origins = conflicting_live_loan(
                            original_points,
                            &cfg.facts,
                            &state,
                            id,
                            &target,
                            LoanAccess::WriteThrough(candidate),
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
            let checked_join_values = if matches!(point.kind, CfgPointKind::Join) {
                cfg.checked_join_values.binary_search_by_key(&id, |&(join, _)| join)
                    .ok().map(|index| cfg.checked_join_values[index].1)
            } else {
                None
            };
            let checked_block_value = matches!(&point.kind, CfgPointKind::Evaluate(ScalarExpression::Block(_)))
                .then(|| cfg.checked_block_values.binary_search_by_key(&id, |&(wrapper, _)| wrapper).ok())
                .flatten()
                .map(|index| cfg.checked_block_values[index].1);
            let checked_value = match &point.kind {
                CfgPointKind::Evaluate(_) if let Some(terminal) = checked_block_value => {
                    state.point_values.get(terminal).cloned()
                }
                CfgPointKind::ReturnOutput { value_point, .. } => {
                    state.point_values.get(*value_point).cloned()
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
                    let origins = unavailable_checked_handles(&state, binding, &[], CheckedHandleStorage::Lexical);
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(),
                            origins,
                        });
                    }
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
                    let rhs = match cfg.checked_binding_initializers
                        .binary_search_by_key(&id, |&(bind, _)| bind)
                    {
                        Ok(index) => state.point_values.get(cfg.checked_binding_initializers[index].1),
                        Err(_) => point.possible_origins.as_ref(),
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
                    reinitialize_checked_handles(&mut state, target, CheckedHandleStorage::Lexical);
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
                    if let CfgPointKind::Borrow { place, loan } = &point.kind {
                        if state.released.contains(&place.binding) {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        }
                        let check = if loan.parent.is_none() {
                            is_lexical_field_path(&place.projections)
                        } else {
                            pending_here.is_some_and(|(_, site)| selected_parent.is_some_and(|(origin, _)|
                                state.point_values.get(site.read).is_some_and(|origins| origins.contains(&origin))))
                                || cfg.named_reborrow_parents
                                .binary_search_by_key(&id, |parent| parent.borrow)
                                .ok()
                                .is_some_and(|index| {
                                    let parent = &cfg.named_reborrow_parents[index];
                                    state.point_values.get(parent.pointer)
                                        .is_some_and(|origins| origins.contains(&parent.origin))
                                })
                        };
                        if check && !state.bindings.is_empty() {
                            let origins = conflicting_live_loan(
                                original_points,
                                &cfg.facts,
                                &state,
                                id,
                                place,
                                LoanAccess::Borrow(loan),
                            );
                            if !origins.is_empty() {
                                errors.push(ReferenceAnalysisError::ConflictingPlace {
                                    source_span: point.source_span.clone(),
                                    origins,
                                });
                            }
                        }
                    }
                    if !isolated_call_output && point.possible_origins.is_none() {
                        state.value.clone()
                    } else {
                        point.possible_origins.clone().filter(|set| !set.is_empty())
                    }
                }
                CfgPointKind::Join if let Some([then_point, else_point]) = checked_join_values => {
                    let terminal = |id: Option<CfgPointId>| {
                        id.and_then(|id| state.point_values.get(id))
                            .filter(|origins| !origins.is_empty())
                    };
                    match (terminal(then_point), terminal(else_point)) {
                        (Some(origins), None) | (None, Some(origins)) => Some(origins.clone()),
                        _ => None,
                    }
                }
                CfgPointKind::Join if point.possible_origins.is_none() => state.value.clone(),
                CfgPointKind::ConditionalOutput {
                    then_point,
                    else_point,
                } => {
                    let then_value = state.point_values.get(*then_point);
                    let else_value = state.point_values.get(*else_point);
                    debug_assert!(then_value.is_none() || else_value.is_none());
                    then_value.or(else_value).cloned()
                }
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
            ) || checked_join_values.is_some() || checked_block_value.is_some() || point.possible_origins.is_none()
            {
                point.possible_origins = checked_value.clone();
            }
            if let Some(descriptor) = descriptor.as_deref_mut() {
                descriptor.observe(point, &state, &cfg.facts);
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
                        .moved_checked_handles
                        .retain(|binding, _| binding.block_span != point.source_span.range);
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
            state
                .point_values
                .record(id, &point.possible_origins, &last_use);
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
            if let (Some(cutoff), Some(last_use), Some(flow)) =
                (common_aggregate_cutoff, &aggregate_deadlines, &mut state.aggregate)
            {
                flow.expire_before(cutoff, last_use);
            }
            let prune_for_successor = |next: &mut ReferenceEdgeState, successor| {
                if common_aggregate_cutoff.is_some_and(|cutoff| cutoff < successor) {
                    if let (Some(last_use), Some(flow)) = (&aggregate_deadlines, &mut next.aggregate) {
                        flow.expire_before(successor, last_use);
                    }
                }
            };
            let mut state = Some(state);
            let mut successors = point.successors.iter().peekable();
            while let Some(successor) = successors.next() {
                let slot = &mut incoming[successor.0];
                if move_alternatives.is_empty() {
                    let mut next = if successors.peek().is_none() {
                        state.take().expect("one state for the final successor")
                    } else {
                        state.as_ref().expect("state for remaining successors").clone()
                    };
                    if successor.0 <= id.0 {
                        next.point_values.clear_from(*successor);
                    }
                    prune_for_successor(&mut next, *successor);
                    if fallback.as_ref().is_none_or(|(visited, _, _)| !visited[successor.0].contains(&next))
                        && !slot.contains(&next) {
                        slot.push(next);
                    }
                } else {
                    for (place, origin) in &move_alternatives {
                        let mut next = state.as_ref().expect("state for move alternatives").clone();
                        if !next.moved.iter().any(|(moved, _)| moved == place) {
                            next.moved.push((place.clone(), origin.clone()));
                        }
                        if successor.0 <= id.0 {
                            next.point_values.clear_from(*successor);
                        }
                        prune_for_successor(&mut next, *successor);
                        if fallback.as_ref().is_none_or(|(visited, _, _)| !visited[successor.0].contains(&next))
                            && !slot.contains(&next) {
                            slot.push(next);
                        }
                    }
                }
                if let Some((_, queued, ready)) = &mut fallback {
                    if !slot.is_empty() && !queued[successor.0] {
                        queued[successor.0] = true;
                        ready.push_back(*successor);
                    }
                }
            }
        }
    }
    for (point, observed) in cfg.points.iter_mut().zip(observed) {
        let Some(mut observed) = observed else { continue };
        observed.kind = match observed.kind {
            CfgPointKind::MoveThrough { place, ty, mut candidates, .. }
                if !matches!(place, ScalarPlace::Dereference { .. })
                    && candidates.len() == 1
                    && fallback.as_ref().map_or_else(
                        || forward_single_observation.as_ref().is_some_and(|single| single[observed.id.0]),
                        |(visited, _, _)| visited[observed.id.0].len() == 1,
                    )
                    && observed.possible_origins.as_ref().is_some_and(|origins| origins.len() == 1) =>
            {
                let candidate = candidates.pop().unwrap();
                CfgPointKind::Move {
                    owner: candidate.place.binding.clone(),
                    place: candidate.place,
                    ty,
                }
            }
            kind => kind,
        };
        *point = observed;
    }
    let mut unique = Vec::new();
    for error in errors {
        if !unique.contains(&error) {
            unique.push(error);
        }
    }
    unique
}

fn synthetic_member_place(
    receiver: &str,
    receiver_span: ByteSpan,
    field: &ScalarStructFieldId,
    span: ByteSpan,
) -> ScalarPlace {
    ScalarPlace::Field {
        base: Box::new(ScalarPlace::Name {
            name: receiver.to_owned(),
            span: receiver_span,
        }),
        field: ScalarFieldReference::Resolved(field.clone()),
        span,
    }
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

#[derive(Default)]
struct CallDeclarationIndex<'a> {
    modules: HashMap<&'a SourceIdentity, CallDeclarationHeaders<'a>>,
    structs: BTreeMap<&'a ScalarStructId, &'a ScalarStruct>,
}

#[derive(Default)]
struct CallDeclarationHeaders<'a> {
    namespaces: HashMap<&'a str, &'a SourceIdentity>,
    functions: HashMap<&'a str, (ByteSpan, CallFunctionKind, (usize, usize))>,
    bindings: HashMap<&'a str, CallBindingHeader<'a>>,
    externs: HashMap<&'a str, HashMap<&'a str, &'a ScalarExternFunction>>,
    value_tokens: HashMap<&'a str, ByteSpan>,
}

struct CallBindingHeader<'a> {
    span: ByteSpan,
    ty: &'a ScalarType,
    module_member: bool,
}

#[derive(Clone, Copy)]
enum CallFunctionKind {
    Ordinary,
    Generic,
    Overload,
}

enum CallDeclaration<'a> {
    Function(
        &'a SourceIdentity,
        ByteSpan,
        CallFunctionKind,
        (usize, usize),
    ),
    Binding(&'a SourceIdentity, ByteSpan),
    Extern(&'a SourceIdentity, &'a ScalarExternFunction),
}

impl<'a> CallDeclarationIndex<'a> {
    fn new(modules: &[CallResolutionModule<'a>]) -> Self {
        let mut index = Self::default();
        for (module_ordinal, module) in modules.iter().enumerate() {
            let mut headers = CallDeclarationHeaders::default();
            for binding in module.namespace_bindings {
                headers
                    .namespaces
                    .entry(&binding.binding)
                    .or_insert(&binding.target);
            }
            for (item_ordinal, item) in module.items.iter().enumerate() {
                match item {
                    ScalarItem::Function(function) => {
                        headers
                            .value_tokens
                            .insert(&function.name, function.name_span);
                        let kind = if !function.overload_arms.is_empty() {
                            CallFunctionKind::Overload
                        } else if !function.generic_parameters.is_empty() {
                            CallFunctionKind::Generic
                        } else {
                            CallFunctionKind::Ordinary
                        };
                        headers.functions.insert(
                            &function.name,
                            (function.name_span, kind, (module_ordinal, item_ordinal)),
                        );
                    }
                    ScalarItem::Binding(binding) => {
                        headers
                            .value_tokens
                            .insert(&binding.name, binding.name_span);
                        headers.bindings.insert(
                            &binding.name,
                            CallBindingHeader {
                                span: binding.name_span,
                                ty: &binding.declared_type,
                                module_member: true,
                            },
                        );
                        for receiver in binding.receivers.iter().skip(1) {
                            headers
                                .value_tokens
                                .insert(&receiver.name, receiver.name_span);
                            headers.bindings.insert(
                                &receiver.name,
                                CallBindingHeader {
                                    span: receiver.name_span,
                                    ty: &receiver.ty,
                                    module_member: false,
                                },
                            );
                        }
                    }
                    ScalarItem::Extern(group) => {
                        headers
                            .value_tokens
                            .insert(&group.binding, group.binding_span);
                        headers.externs.entry(&group.binding).or_insert_with(|| {
                            let mut members = HashMap::new();
                            for function in &group.functions {
                                members
                                    .entry(function.name.as_str())
                                    .or_insert(function);
                            }
                            members
                        });
                    }
                    ScalarItem::Namespace(namespace) => {
                        headers
                            .value_tokens
                            .insert(&namespace.binding, namespace.binding_span);
                    }
                    ScalarItem::Executable(_) => {}
                }
            }
            index.modules.entry(module.source).or_insert(headers);
            for structure in module.structs {
                index.structs.entry(&structure.id).or_insert(structure);
            }
        }
        index
    }

    fn callable(
        &self,
        source: &'a SourceIdentity,
        receiver: Option<&str>,
        name: &str,
    ) -> Option<CallDeclaration<'a>> {
        let local = self.modules.get(source)?;
        let (source, headers) = match receiver {
            None => (source, local),
            Some(receiver) => match local.namespaces.get(receiver) {
                Some(target) if !is_module_private_name(name) => {
                    (*target, self.modules.get(target)?)
                }
                Some(_) => return None,
                None => {
                    return Some(CallDeclaration::Extern(
                        source,
                        *local.externs.get(receiver)?.get(name)?,
                    ))
                }
            },
        };
        if let Some(&(span, kind, origin)) = headers.functions.get(name) {
            return Some(CallDeclaration::Function(source, span, kind, origin));
        }
        let binding = headers.bindings.get(name)?;
        if !matches!(binding.ty, ScalarType::Callable { .. })
            || (receiver.is_some() && !binding.module_member)
        {
            return None;
        }
        Some(CallDeclaration::Binding(source, binding.span))
    }

    fn binding(&self, source: &SourceIdentity, name: &str) -> Option<(ByteSpan, &'a ScalarType)> {
        let binding = self.modules.get(source)?.bindings.get(name)?;
        Some((binding.span, binding.ty))
    }

    fn value_token(&self, source: &SourceIdentity, name: &str) -> Option<ByteSpan> {
        self.modules.get(source)?.value_tokens.get(name).copied()
    }

    fn has_value(&self, source: &SourceIdentity, name: &str) -> bool {
        self.modules.get(source).is_some_and(|headers| {
            headers.bindings.contains_key(name) || headers.functions.contains_key(name)
        })
    }

    fn field(&self, ty: &ScalarType, name: &str) -> Option<&'a ScalarStructField> {
        let ScalarType::Struct(id) = ty else {
            return None;
        };
        resolved_struct_field(ty, name, std::slice::from_ref(*self.structs.get(id)?))
    }
}

#[cfg(test)]
pub(super) fn resolve_program_call_sites(
    program: &ScalarProgram,
) -> Result<ModuleCallSites, ReferenceAnalysisError> {
    resolve_program_call_sites_impl(program, false, PreparationOccurrenceTyping::default())
}

pub(super) fn resolve_program_call_sites_with_occurrences(
    program: &ScalarProgram,
    occurrence_typing: PreparationOccurrenceTyping,
) -> Result<ModuleCallSites, ReferenceAnalysisError> {
    resolve_program_call_sites_impl(program, true, occurrence_typing)
}

fn resolve_program_call_sites_impl(
    program: &ScalarProgram,
    select_executable_calls: bool,
    occurrence_typing: PreparationOccurrenceTyping,
) -> Result<ModuleCallSites, ReferenceAnalysisError> {
    let declarations = CallDeclarationIndex::new(&[CallResolutionModule::program(program)]);
    resolve_module_call_sites(
        &program.source,
        &program.items,
        &[],
        &declarations,
        program.resolved_assignment_outputs.as_map(),
        select_executable_calls,
        occurrence_typing,
    )
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ModuleCallSites {
    functions: BTreeMap<usize, FunctionCallSites>,
    pub(super) initialization: ModuleInitializationRegion,
}

impl ModuleCallSites {
    pub(super) fn function_body(
        &self,
        item: usize,
        arm: Option<usize>,
    ) -> Option<&FunctionCallSites> {
        let function = self.functions.get(&item)?;
        match arm {
            Some(arm) => function.arms.get(arm),
            None => Some(function),
        }
    }

    pub(super) fn functions_in_item_order(&self) -> impl Iterator<Item = &FunctionCallSites> {
        self.functions.values()
    }
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
pub(crate) struct OccurrenceOrdinal(pub(super) usize);

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
    BindingAllocationLength { item: usize, position: usize },
    QueryAllocationPlaceholder { item: usize, output: usize },
    ExpressionItem { item: usize },
    FinalOutput { item: usize, output: usize },
    AssignmentRhs { item: usize, output: usize },
    AssignmentDestination { item: usize, destination: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OccurrenceNode {
    Expression(OccurrenceOrdinal),
    Place(OccurrenceOrdinal),
    Block(BlockOrdinal),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModuleStorageRole {
    Read,
    Argument { call: OccurrenceOrdinal, slot: usize },
    Project,
    Address,
    Replace,
    CallableReceiver,
}

impl ModuleStorageRole {
    fn value(parent: OccurrenceParent, edge: OccurrenceEdge) -> Self {
        match (parent, edge) {
            (OccurrenceParent::Expression(call), OccurrenceEdge::CallArgument(slot)) =>
                Self::Argument { call, slot },
            _ => Self::Read,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ModuleStorageInstall {
    pub(super) storage: SourceSpan,
    pub(super) producer: OccurrenceOrdinal,
    pub(super) consumer: Option<OccurrenceOrdinal>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OccurrenceStructure {
    pub(super) root: BlockOrdinal,
    pub(super) blocks: Vec<BlockRegion>,
    pub(super) edges: HashMap<(OccurrenceParent, OccurrenceEdge), OccurrenceNode>,
    pub(super) checked_types: CheckedOccurrenceTypes,
    pub(super) literal_fields: HashMap<OccurrenceOrdinal, ScalarStructFieldId>,
    pub(super) module_storage_uses: HashMap<SourceSpan, Vec<(OccurrenceOrdinal, ModuleStorageRole)>>,
    pub(super) module_storage_installs: BTreeMap<usize, ModuleStorageInstall>,
    pub(super) initializer_static_utf8: HashSet<OccurrenceOrdinal>,
    pub(super) initializer_immediate: HashMap<OccurrenceOrdinal, OccurrenceOrdinal>,
    pub(super) resolved_namespace_places: HashMap<OccurrenceOrdinal, SourceIdentity>,
    pub(super) folded_expressions: HashMap<OccurrenceOrdinal, FoldedOccurrence>,
    pub(super) synthetic_expressions: HashMap<OccurrenceOrdinal, SyntheticOccurrence>,
    pub(super) conditional_output_projections:
        HashMap<OccurrenceOrdinal, ConditionalOutputProjection>,
}

impl OccurrenceStructure {
    fn new(span: ByteSpan) -> Self {
        Self {
            root: BlockOrdinal(0),
            blocks: vec![BlockRegion::new(span, Vec::new())],
            edges: HashMap::new(),
            checked_types: CheckedOccurrenceTypes::default(),
            literal_fields: HashMap::new(),
            module_storage_uses: HashMap::new(),
            module_storage_installs: BTreeMap::new(),
            initializer_static_utf8: HashSet::new(),
            initializer_immediate: HashMap::new(),
            resolved_namespace_places: HashMap::new(),
            folded_expressions: HashMap::new(),
            synthetic_expressions: HashMap::new(),
            conditional_output_projections: HashMap::new(),
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
    pub(super) requires_plain_name: bool,
    pub(super) requires_direct_destination: bool,
    pub(super) requires_named_place_root: bool,
    pub(super) initializer_member: InitializerMemberRequirement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InitializerMemberRequirement {
    NotRequired,
    Token(ByteSpan),
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
pub(crate) enum PlainNameIdentity {
    Lexical(ReferenceBindingId),
    ModuleValue(SourceSpan),
    Null,
}

#[derive(Clone, Debug, PartialEq)]
enum PlainNameFacts {
    Raw,
    Prepared(HashMap<OccurrenceOrdinal, PlainNameIdentity>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NamedPlaceRootIdentity {
    Lexical(ReferenceBindingId),
    ModuleValue(SourceSpan),
}

#[derive(Clone, Debug, PartialEq)]
enum NamedPlaceRootFacts {
    Raw,
    Prepared(HashMap<OccurrenceOrdinal, NamedPlaceRootIdentity>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DirectDestinationIdentity {
    Existing(ReferenceBindingId),
    Module(SourceSpan),
    Introduce {
        binding: ReferenceBindingId,
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
enum DirectDestinationFacts {
    Raw,
    Prepared(HashMap<OccurrenceOrdinal, DirectDestinationIdentity>),
}

#[derive(Clone, Debug, PartialEq)]
enum PreparedMemberIdentity {
    Lexical {
        binding: ReferenceBindingId,
        field: ScalarStructFieldId,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq)]
enum MemberFacts {
    Raw,
    Prepared(HashMap<OccurrenceOrdinal, PreparedMemberIdentity>),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FunctionCallSites {
    pub(super) boundary: CallableBoundary,
    pub(super) sites: Vec<ResolvedCallSite>,
    pub(super) arms: Vec<FunctionCallSites>,
    pub(super) occurrences: Vec<OccurrenceMetadata>,
    pub(super) call_occurrences: Vec<CallOccurrence>,
    declared_unsafe_calls: BTreeSet<usize>,
    pub(super) callable_escapes: HashSet<SourceSpan>,
    overload_reservation_sites: Vec<CallOccurrence>,
    pub(super) self_tail_candidates: Vec<OccurrenceOrdinal>,
    pub(super) allocation_length_calls: Vec<OccurrenceOrdinal>,
    pub(super) allocation_length_call_targets: HashMap<OccurrenceOrdinal, ResolvedCallTarget>,
    pub(super) checked_call_signatures: HashMap<OccurrenceOrdinal, CheckedCallSignature>,
    plain_names: PlainNameFacts,
    named_place_roots: NamedPlaceRootFacts,
    direct_destinations: DirectDestinationFacts,
    members: MemberFacts,
    query_allocation_outputs: HashMap<(BlockOrdinal, usize), BindingOutputs>,
    pub(super) structure: OccurrenceStructure,
    pub(super) assignments: HashMap<(BlockOrdinal, usize), AssignmentGroup>,
    pub(super) conditionals: HashMap<OccurrenceOrdinal, ConditionalRegion>,
    pub(super) short_circuits: HashMap<OccurrenceOrdinal, ShortCircuitRegion>,
}

impl FunctionCallSites {
    pub(super) fn overload_reservations(&self) -> impl Iterator<Item = &ResolvedCallTarget> {
        self.overload_reservation_sites
            .iter()
            .map(|call| &self.sites[call.site].target)
    }

    pub(super) fn prepared_plain_name(
        &self,
        ordinal: OccurrenceOrdinal,
    ) -> Option<&PlainNameIdentity> {
        match &self.plain_names {
            PlainNameFacts::Prepared(rows) => rows.get(&ordinal),
            PlainNameFacts::Raw => None,
        }
    }

    pub(super) fn prepared_lexical_member(
        &self,
        ordinal: OccurrenceOrdinal,
    ) -> Option<(&ReferenceBindingId, &ScalarStructFieldId)> {
        let MemberFacts::Prepared(rows) = &self.members else {
            return None;
        };
        match rows.get(&ordinal)? {
            PreparedMemberIdentity::Lexical { binding, field } => Some((binding, field)),
            PreparedMemberIdentity::Other => None,
        }
    }

    pub(super) fn prepared_direct_destination(
        &self,
        ordinal: OccurrenceOrdinal,
    ) -> Option<&DirectDestinationIdentity> {
        match &self.direct_destinations {
            DirectDestinationFacts::Prepared(rows) => rows.get(&ordinal),
            DirectDestinationFacts::Raw => None,
        }
    }

    pub(super) fn has_prepared_plain_name(&self, ordinal: OccurrenceOrdinal) -> bool {
        matches!(&self.plain_names, PlainNameFacts::Prepared(rows) if rows.contains_key(&ordinal))
    }

    pub(super) fn has_prepared_direct_destination(&self, ordinal: OccurrenceOrdinal) -> bool {
        matches!(&self.direct_destinations, DirectDestinationFacts::Prepared(rows) if rows.contains_key(&ordinal))
    }

    pub(super) fn has_prepared_named_place_root(&self, ordinal: OccurrenceOrdinal) -> bool {
        self.prepared_named_place_root(ordinal).is_some()
    }

    pub(super) fn prepared_named_place_root(
        &self,
        ordinal: OccurrenceOrdinal,
    ) -> Option<&NamedPlaceRootIdentity> {
        match &self.named_place_roots {
            NamedPlaceRootFacts::Prepared(rows) => rows.get(&ordinal),
            NamedPlaceRootFacts::Raw => None,
        }
    }
}

pub(super) struct ProjectCallSiteResults {
    modules: Vec<Result<ModuleCallSites, ReferenceAnalysisError>>,
}

#[cfg(test)]
impl ProjectCallSiteResults {
    pub(super) fn module(&self, ordinal: usize) -> &ModuleCallSites {
        self.modules[ordinal].as_ref().unwrap()
    }
}

impl ProjectCallSiteResults {
    pub(super) fn lookup_errors(&self) -> impl Iterator<Item = &ReferenceAnalysisError> {
        self.modules
            .iter()
            .filter_map(|module| module.as_ref().err())
    }

    #[cfg(test)]
    pub(super) fn remove_body_allocation_length_target(
        &mut self,
        module: usize,
        item: usize,
        arm: usize,
    ) -> (ByteSpan, usize) {
        let calls = &mut self.modules[module]
            .as_mut()
            .unwrap()
            .functions
            .get_mut(&item)
            .unwrap()
            .arms[arm];
        let ordinal = calls.allocation_length_calls[0];
        calls
            .allocation_length_call_targets
            .remove(&ordinal)
            .unwrap();
        (calls.occurrences[ordinal.0].span, ordinal.0)
    }

    #[cfg(test)]
    pub(super) fn function(&self, module: usize, item: usize) -> &FunctionCallSites {
        self.function_body(module, item, None).unwrap()
    }

    pub(super) fn function_body(
        &self,
        module_ordinal: usize,
        item_ordinal: usize,
        arm_index: Option<usize>,
    ) -> Option<&FunctionCallSites> {
        let module = self.modules.get(module_ordinal)?.as_ref().ok()?;
        let function = module.functions.get(&item_ordinal)?;
        match arm_index {
            Some(arm) => function.arms.get(arm),
            None => Some(function),
        }
    }

    pub(super) fn initialization_region(
        &self,
        module_ordinal: usize,
    ) -> Option<&ModuleInitializationRegion> {
        let calls = self.modules.get(module_ordinal)?.as_ref().ok()?;
        Some(&calls.initialization)
    }
}

#[cfg(test)]
pub(super) fn resolve_project_call_sites(modules: &[ScalarModule]) -> ProjectCallSiteResults {
    resolve_project_call_sites_impl(modules, OccurrenceTypingInputs::StructuralOnly)
}

pub(super) fn resolve_project_call_sites_with_occurrences(
    modules: &[ScalarModule],
    occurrence_typing: Vec<PreparationOccurrenceTyping>,
) -> ProjectCallSiteResults {
    assert_eq!(modules.len(), occurrence_typing.len());
    resolve_project_call_sites_impl(
        modules,
        OccurrenceTypingInputs::Typed(occurrence_typing.into_iter()),
    )
}

enum OccurrenceTypingInputs {
    Typed(std::vec::IntoIter<PreparationOccurrenceTyping>),
    #[cfg(test)]
    StructuralOnly,
}

fn resolve_project_call_sites_impl(
    modules: &[ScalarModule],
    mut occurrence_inputs: OccurrenceTypingInputs,
) -> ProjectCallSiteResults {
    let views = modules
        .iter()
        .map(CallResolutionModule::module)
        .collect::<Vec<_>>();
    let declarations = CallDeclarationIndex::new(&views);
    ProjectCallSiteResults {
        modules: modules
            .iter()
            .map(|module| {
                let (select_executable_calls, occurrence_typing) = match &mut occurrence_inputs {
                    OccurrenceTypingInputs::Typed(occurrences) => (
                        true,
                        occurrences.next().expect("one typing table per module"),
                    ),
                    #[cfg(test)]
                    OccurrenceTypingInputs::StructuralOnly => {
                        (false, PreparationOccurrenceTyping::default())
                    }
                };
                resolve_module_call_sites(
                    &module.source,
                    &module.items,
                    &module.namespace_bindings,
                    &declarations,
                    module.resolved_assignment_outputs.as_map(),
                    select_executable_calls,
                    occurrence_typing,
                )
            })
            .collect(),
    }
}

#[derive(Eq, PartialEq)]
struct CallSiteSpan(ByteSpan);

impl Ord for CallSiteSpan {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        #[cfg(test)]
        call_site_comparisons::record();
        (self.0.start, self.0.end).cmp(&(other.0.start, other.0.end))
    }
}

impl PartialOrd for CallSiteSpan {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy)]
struct CallableTargetResolver<'a> {
    source: &'a SourceIdentity,
    function_span: ByteSpan,
    declarations: &'a CallDeclarationIndex<'a>,
}

struct CallableWalk<'a> {
    target_resolver: CallableTargetResolver<'a>,
    prepared: bool,
    assignment_outputs: &'a HashMap<ResolvedAssignmentOutputId, ScalarType>,
    sites: Vec<ResolvedCallSite>,
    site_by_span: BTreeMap<CallSiteSpan, usize>,
    call_occurrences: Vec<CallOccurrence>,
    declared_unsafe_calls: BTreeSet<usize>,
    overload_reservation_sites: Vec<CallOccurrence>,
    query_allocation_outputs: HashMap<(BlockOrdinal, usize), BindingOutputs>,
}

enum WalkMode<'a> {
    Callable(CallableWalk<'a>),
    CallableAllocationLength(CallableTargetResolver<'a>),
    ModuleInitialization {
        source: &'a SourceIdentity,
        declarations: &'a CallDeclarationIndex<'a>,
    },
    StructuralOnly,
}

#[derive(Clone, Copy)]
struct WalkBinding {
    declaration_span: ByteSpan,
    block_span: ByteSpan,
    introduced_by_assignment: bool,
}

impl WalkBinding {
    fn initializer_id(self) -> InitializationLocal {
        InitializationLocal {
            declaration_span: self.declaration_span,
            block_span: self.block_span,
        }
    }

    fn reference_id(self, source: &SourceIdentity, function_span: ByteSpan) -> ReferenceBindingId {
        ReferenceBindingId {
            source: source.clone(),
            function_span,
            declaration_span: self.declaration_span,
            block_span: self.block_span,
            kind: ReferenceBindingKind::Declared,
        }
    }
}

fn initializer_storage(
    name: &str,
    names: &BTreeMap<String, WalkBinding>,
    source: &SourceIdentity,
    declarations: &CallDeclarationIndex<'_>,
) -> Option<InitializationStorage> {
    if let Some(binding) = names.get(name) {
        Some(InitializationStorage::Lexical(binding.initializer_id()))
    } else {
        declarations
            .value_token(source, name)
            .map(InitializationStorage::Module)
    }
}

#[derive(Default)]
struct WalkScope {
    names: BTreeMap<String, WalkBinding>,
    types: BTreeMap<String, ScalarType>,
}

struct StructuralWalker<'a, 'typing> {
    mode: WalkMode<'a>,
    occurrence_typing: &'typing mut PreparationOccurrenceTyping,
    checked_call_signatures: HashMap<OccurrenceOrdinal, CheckedCallSignature>,
    plain_names: PlainNameFacts,
    named_place_roots: NamedPlaceRootFacts,
    direct_destinations: DirectDestinationFacts,
    members: MemberFacts,
    allocation_length_call_targets: HashMap<OccurrenceOrdinal, ResolvedCallTarget>,
    initialization_call_targets: HashMap<OccurrenceOrdinal, ModuleInitializationCallTarget>,
    initialization_identities: InitializationIdentities,
    callable_escapes: HashSet<SourceSpan>,
    initialization_reservations: Vec<(usize, OccurrenceOrdinal, bool)>,
    initialization_item: usize,
    reserve_legacy_blocks: Vec<bool>,
    reserve_legacy_occurrences: Vec<bool>,
    structural_calls: Vec<OccurrenceOrdinal>,
    occurrences: Vec<OccurrenceMetadata>,
    structure: OccurrenceStructure,
    assignments: HashMap<(BlockOrdinal, usize), AssignmentGroup>,
    conditionals: HashMap<OccurrenceOrdinal, ConditionalRegion>,
    short_circuits: HashMap<OccurrenceOrdinal, ShortCircuitRegion>,
}

impl<'a, 'typing> StructuralWalker<'a, 'typing> {
    fn new(
        mode: WalkMode<'a>,
        span: ByteSpan,
        occurrence_typing: &'typing mut PreparationOccurrenceTyping,
    ) -> Self {
        let (plain_names, named_place_roots, members) = match &mode {
            WalkMode::Callable(callable) if callable.prepared => (
                PlainNameFacts::Prepared(HashMap::new()),
                NamedPlaceRootFacts::Prepared(HashMap::new()),
                MemberFacts::Prepared(HashMap::new()),
            ),
            _ => (
                PlainNameFacts::Raw,
                NamedPlaceRootFacts::Raw,
                MemberFacts::Raw,
            ),
        };
        Self {
            direct_destinations: match &mode {
                WalkMode::Callable(callable) if callable.prepared => {
                    DirectDestinationFacts::Prepared(HashMap::new())
                }
                _ => DirectDestinationFacts::Raw,
            },
            mode,
            occurrence_typing,
            checked_call_signatures: HashMap::new(),
            plain_names,
            named_place_roots,
            members,
            allocation_length_call_targets: HashMap::new(),
            initialization_call_targets: HashMap::new(),
            initialization_identities: InitializationIdentities::default(),
            callable_escapes: HashSet::new(),
            initialization_reservations: Vec::new(),
            initialization_item: 0,
            reserve_legacy_blocks: vec![true],
            reserve_legacy_occurrences: Vec::new(),
            structural_calls: Vec::new(),
            occurrences: Vec::new(),
            structure: OccurrenceStructure::new(span),
            assignments: HashMap::new(),
            conditionals: HashMap::new(),
            short_circuits: HashMap::new(),
        }
    }

    fn storage_use(
        structure: &mut OccurrenceStructure,
        declarations: &CallDeclarationIndex<'_>,
        source: &SourceIdentity,
        name: &str,
        ordinal: OccurrenceOrdinal,
        role: ModuleStorageRole,
    ) {
        let Some((span, ScalarType::Struct(_))) = declarations.binding(source, name) else {
            return;
        };
        structure.module_storage_uses.entry(SourceSpan::new(source.clone(), span))
            .or_default().push((ordinal, role));
    }

    fn reserve_legacy(&self, parent: OccurrenceParent, edge: OccurrenceEdge) -> bool {
        let admitted = match parent {
            OccurrenceParent::Block(block) => self.reserve_legacy_blocks[block.0],
            OccurrenceParent::Expression(expression) | OccurrenceParent::Place(expression) => {
                self.reserve_legacy_occurrences[expression.0]
            }
        };
        admitted
            && !matches!(
                edge,
                OccurrenceEdge::BindingRoot { output: 1.., .. }
                    | OccurrenceEdge::BindingAllocationLength { .. }
                    | OccurrenceEdge::QueryAllocationPlaceholder { .. }
                    | OccurrenceEdge::AssignmentRhs { output: 1.., .. }
                    | OccurrenceEdge::AssignmentDestination { .. }
            )
    }

    fn record_occurrence(
        &mut self,
        role: OccurrenceRole,
        span: ByteSpan,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
    ) -> OccurrenceOrdinal {
        let ordinal = OccurrenceOrdinal(self.occurrences.len());
        self.reserve_legacy_occurrences
            .push(self.reserve_legacy(parent, edge));
        self.occurrences.push(OccurrenceMetadata {
            role,
            span,
            requires_plain_name: false,
            requires_direct_destination: false,
            requires_named_place_root: false,
            initializer_member: InitializerMemberRequirement::NotRequired,
        });
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
        names: &BTreeMap<String, WalkBinding>,
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
        names: &BTreeMap<String, WalkBinding>,
        types: &BTreeMap<String, ScalarType>,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
    ) -> Result<OccurrenceOrdinal, ReferenceAnalysisError> {
        self.expression_at_with_initializer_statement(expression, names, types, parent, edge, false)
    }

    fn expression_at_with_initializer_statement(
        &mut self,
        expression: &ScalarExpression,
        names: &BTreeMap<String, WalkBinding>,
        types: &BTreeMap<String, ScalarType>,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
        initializer_statement: bool,
    ) -> Result<OccurrenceOrdinal, ReferenceAnalysisError> {
        let ordinal = self.record_occurrence(
            OccurrenceRole::Expression,
            expression_span(expression),
            parent,
            edge,
        );
        if matches!(
            expression,
            ScalarExpression::RawAddress { .. } | ScalarExpression::CheckedAddress { .. }
        ) {
            self.reserve_legacy_occurrences[ordinal.0] = false;
        }
        let mut typed_callable = false;
        if let Some(ty) = self.occurrence_typing.take_expression_type(expression) {
            typed_callable = matches!(&ty,
                ScalarType::Callable { outputs, .. } if matches!(outputs.outputs.as_slice(),
                    [ScalarOutput { ty: ScalarType::Struct(_), .. }]
                )
            );
            self.structure.checked_types.expressions.insert(ordinal, ty);
        }
        if matches!(&self.mode, WalkMode::ModuleInitialization { .. })
            && matches!(expression, ScalarExpression::Utf8 { .. })
            && matches!(edge, OccurrenceEdge::CallArgument(_))
            && matches!(self.structure.checked_types.expressions.get(&ordinal), Some(ScalarType::Struct(_)))
        {
            self.structure.initializer_static_utf8.insert(ordinal);
        }
        if let Some(folded) = self.occurrence_typing.take_folded_expression(expression) {
            debug_assert_eq!(edge, OccurrenceEdge::UnaryOperand);
            self.structure.folded_expressions.insert(ordinal, folded);
        }
        if let (ScalarExpression::Name { name, .. }, PlainNameFacts::Prepared(identities)) =
            (expression, &mut self.plain_names)
        {
            self.occurrences[ordinal.0].requires_plain_name = true;
            let resolver = match &self.mode {
                WalkMode::Callable(callable) => callable.target_resolver,
                WalkMode::CallableAllocationLength(resolver) => *resolver,
                _ => unreachable!("plain name facts are owned by a prepared callable"),
            };
            let identity = if name == "null" {
                Some(PlainNameIdentity::Null)
            } else {
                let module = || resolver.declarations.value_token(resolver.source, name);
                match names.get(name) {
                    Some(binding) if !binding.introduced_by_assignment => {
                        Some(PlainNameIdentity::Lexical(
                            binding.reference_id(resolver.source, resolver.function_span),
                        ))
                    }
                    binding => match module() {
                        Some(span) => Some(PlainNameIdentity::ModuleValue(SourceSpan::new(
                            resolver.source.clone(),
                            span,
                        ))),
                        None => binding.map(|binding| {
                            PlainNameIdentity::Lexical(
                                binding.reference_id(resolver.source, resolver.function_span),
                            )
                        }),
                    },
                }
            };
            if let Some(identity) = identity {
                if let PlainNameIdentity::ModuleValue(target) = &identity {
                    if typed_callable {
                        self.callable_escapes.insert(target.clone());
                    }
                    Self::storage_use(&mut self.structure, resolver.declarations, &target.source,
                        name, ordinal, ModuleStorageRole::value(parent, edge));
                }
                identities.insert(ordinal, identity);
            }
        }
        if let (
            ScalarExpression::Name { name, .. },
            WalkMode::ModuleInitialization {
                source,
                declarations,
            },
        ) = (expression, &self.mode)
        {
            self.occurrences[ordinal.0].requires_plain_name = true;
            let identity = if name == "null" {
                Some(InitializationName::Null)
            } else {
                initializer_storage(name, names, source, declarations)
                    .map(InitializationName::Storage)
            };
            if let Some(identity) = identity {
                if let InitializationName::Storage(InitializationStorage::Module(span)) = identity {
                    if typed_callable {
                        self.callable_escapes.insert(SourceSpan::new((*source).clone(), span));
                    }
                    Self::storage_use(&mut self.structure, declarations, source, name, ordinal,
                        ModuleStorageRole::value(parent, edge));
                }
                self.initialization_identities.names.insert(ordinal, identity);
            }
        }
        if let (ScalarExpression::Member { receiver, .. }, MemberFacts::Prepared(identities)) =
            (expression, &mut self.members)
        {
            let resolver = match &self.mode {
                WalkMode::Callable(callable) => callable.target_resolver,
                WalkMode::CallableAllocationLength(resolver) => *resolver,
                _ => unreachable!("member facts are owned by a prepared callable"),
            };
            let lexical = names.get(receiver).filter(|binding| {
                !binding.introduced_by_assignment
                    || !resolver.declarations.has_value(resolver.source, receiver)
            });
            let selection = self.occurrence_typing.take_member(expression);
            match &selection {
                Some(CheckedMember::Module(target)) => {
                    let ScalarExpression::Member { name, .. } = expression else {
                        unreachable!("prepared member occurrence");
                    };
                    if typed_callable {
                        if let Some(span) = resolver.declarations.value_token(target, name) {
                            self.callable_escapes.insert(SourceSpan::new(target.clone(), span));
                        }
                    }
                    Self::storage_use(&mut self.structure, resolver.declarations, target, name,
                        ordinal, ModuleStorageRole::value(parent, edge));
                }
                Some(CheckedMember::DirectField(_) | CheckedMember::RawPointerField(_)) if lexical.is_none() =>
                    Self::storage_use(&mut self.structure, resolver.declarations, resolver.source, receiver,
                        ordinal, ModuleStorageRole::Project),
                _ => {}
            }
            let identity = match (selection, lexical) {
                (Some(CheckedMember::DirectField(field)), Some(binding)) => {
                    Some(PreparedMemberIdentity::Lexical {
                        binding: binding.reference_id(resolver.source, resolver.function_span),
                        field,
                    })
                }
                (Some(_), _) => Some(PreparedMemberIdentity::Other),
                (None, _) => None,
            };
            if let Some(identity) = identity {
                identities.insert(ordinal, identity);
            }
        }
        if let (
            ScalarExpression::Member {
                receiver,
                name,
                name_span,
                ..
            },
            WalkMode::ModuleInitialization {
                source,
                declarations,
            },
        ) = (expression, &self.mode)
        {
            self.occurrences[ordinal.0].initializer_member =
                InitializerMemberRequirement::Token(*name_span);
            let selection = self.occurrence_typing.take_member(expression);
            let identity = match selection {
                Some(
                    CheckedMember::DirectField(ref field)
                    | CheckedMember::RawPointerField(ref field),
                ) => initializer_storage(receiver, names, source, declarations).map(|storage| {
                    InitializationMember::Field {
                        receiver: storage,
                        receiver_name: receiver.clone(),
                        field: field.clone(),
                        through_raw_pointer: matches!(
                            selection,
                            Some(CheckedMember::RawPointerField(_))
                        ),
                    }
                }),
                Some(CheckedMember::Module(target)) => declarations
                    .value_token(source, receiver)
                    .and_then(|qualifier| {
                        declarations.value_token(&target, name).map(|token| {
                            InitializationMember::Module {
                                qualifier,
                                declaration: SourceSpan::new(target, token),
                                name: name.clone(),
                            }
                        })
                    }),
                Some(CheckedMember::Enum { enumeration, tag }) => {
                    Some(InitializationMember::Enum { enumeration, tag })
                }
                None => None,
            };
            if let Some(identity) = identity {
                match &identity {
                    InitializationMember::Module { declaration, .. } => Self::storage_use(
                        &mut self.structure, declarations, &declaration.source, name, ordinal,
                        ModuleStorageRole::value(parent, edge)),
                    InitializationMember::Field { receiver: InitializationStorage::Module(_), .. } =>
                        Self::storage_use(&mut self.structure, declarations, source, receiver, ordinal,
                            ModuleStorageRole::Project),
                    _ => {}
                }
                if typed_callable {
                    if let InitializationMember::Module { declaration, .. } = &identity {
                        self.callable_escapes.insert(declaration.clone());
                    }
                }
                self.initialization_identities
                    .members
                    .insert(ordinal, identity);
            }
        }
        self.expression_children(expression, names, types, ordinal, initializer_statement)?;
        Ok(ordinal)
    }
}

impl CallableTargetResolver<'_> {
    fn resolve(
        &self,
        expression: &ScalarExpression,
        names: &BTreeMap<String, WalkBinding>,
        types: &BTreeMap<String, ScalarType>,
        signature: Option<&CheckedCallSignature>,
    ) -> Result<(ResolvedCallTarget, bool), ReferenceAnalysisError> {
        let ScalarExpression::Call {
            receiver,
            receiver_span,
            name,
            span,
            ..
        } = expression
        else {
            unreachable!("callable registration requires a call occurrence")
        };
        let missing = || missing_call_target(self.source, *span);
        let mut declared_unsafe = false;
        let target = if receiver.as_deref() == Some("core") {
            CoreOperationId::from_name(name).map(ResolvedCallTarget::Core)
        } else if let Some(binding) = receiver.is_none().then(|| names.get(name)).flatten() {
            Some(ResolvedCallTarget::LocalCallable(
                binding.reference_id(self.source, self.function_span),
            ))
        } else if let Some((binding_name, receiver_type)) = receiver
            .as_ref()
            .and_then(|binding| Some((binding, types.get(binding)?)))
        {
            let field = self
                .declarations
                .field(receiver_type, name)
                .ok_or_else(missing)?;
            let binding = names.get(binding_name).ok_or_else(missing)?;
            let receiver_span = receiver_span.ok_or_else(missing)?;
            Some(ResolvedCallTarget::CallableField {
                receiver_place: ReferencePlaceId {
                    source: self.source.clone(),
                    function_span: self.function_span,
                    declaration_span: binding.declaration_span,
                    block_span: binding.block_span,
                    binding: binding.reference_id(self.source, self.function_span),
                    projections: Vec::new(),
                    place: ScalarPlace::Name {
                        name: binding_name.clone(),
                        span: receiver_span,
                    },
                },
                field: field.id.clone(),
            })
        } else {
            self.declarations
                .callable(self.source, receiver.as_deref(), name)
                .and_then(|declaration| match declaration {
                    CallDeclaration::Function(
                        source,
                        declaration_span,
                        CallFunctionKind::Overload,
                        origin,
                    ) => Some(ResolvedCallTarget::ModuleCallable {
                        source: source.clone(),
                        declaration_span,
                        concrete: ConcreteCallSelection::Overload(
                            checked_overload_selection(signature, origin)?.clone(),
                        ),
                    }),
                    CallDeclaration::Function(source, declaration_span, _, _)
                    | CallDeclaration::Binding(source, declaration_span) => {
                        Some(ResolvedCallTarget::ModuleCallable {
                            source: source.clone(),
                            declaration_span,
                            concrete: ConcreteCallSelection::Declaration,
                        })
                    }
                    CallDeclaration::Extern(source, declaration) => {
                        declared_unsafe = declaration.unsafe_marker;
                        Some(ResolvedCallTarget::ExternCallable {
                            source: source.clone(),
                            declaration_span: declaration.name_span,
                        })
                    }
                })
        };
        target.map(|target| (target, declared_unsafe)).ok_or_else(missing)
    }
}

impl CallableWalk<'_> {
    fn register_call(
        &mut self,
        ordinal: OccurrenceOrdinal,
        expression: &ScalarExpression,
        names: &BTreeMap<String, WalkBinding>,
        types: &BTreeMap<String, ScalarType>,
        signature: Option<&CheckedCallSignature>,
        reserve_legacy: bool,
    ) -> Result<(), ReferenceAnalysisError> {
        use std::collections::btree_map::Entry;

        let (target, declared_unsafe) = self
            .target_resolver
            .resolve(expression, names, types, signature)?;
        if declared_unsafe {
            self.declared_unsafe_calls.insert(ordinal.0);
        }
        let span = expression_span(expression);
        let source = self.target_resolver.source;
        let site_index = match self.site_by_span.entry(CallSiteSpan(span)) {
            Entry::Occupied(entry) => {
                let index = *entry.get();
                if self.sites[index].target != target {
                    return Err(missing_call_target(source, span));
                }
                index
            }
            Entry::Vacant(entry) => {
                let index = self.sites.len();
                self.sites.push(ResolvedCallSite {
                    source_span: SourceSpan::new(source.clone(), span),
                    target,
                });
                entry.insert(index);
                index
            }
        };
        if reserve_legacy
            && matches!(
                self.sites[site_index].target,
                ResolvedCallTarget::ModuleCallable {
                    concrete: ConcreteCallSelection::Overload(_),
                    ..
                }
            )
        {
            self.overload_reservation_sites.push(CallOccurrence {
                ordinal,
                site: site_index,
            });
        }
        self.call_occurrences.push(CallOccurrence {
            ordinal,
            site: site_index,
        });
        Ok(())
    }
}

fn missing_call_target(source: &SourceIdentity, span: ByteSpan) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: SourceSpan::new(source.clone(), span),
        reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
    }
}

fn checked_overload_selection(
    signature: Option<&CheckedCallSignature>,
    origin: (usize, usize),
) -> Option<&ScalarOverloadSelection> {
    signature
        .filter(|row| row.overload_origin == Some(origin))?
        .overload
        .as_ref()
}

fn initialization_call_target(
    source: &SourceIdentity,
    declarations: &CallDeclarationIndex<'_>,
    expression: &ScalarExpression,
    names: &BTreeMap<String, WalkBinding>,
    types: &BTreeMap<String, ScalarType>,
    signature: Option<&CheckedCallSignature>,
) -> Result<ModuleInitializationCallTarget, ReferenceAnalysisError> {
    use ModuleInitializationCallTarget as Target;
    let ScalarExpression::Call {
        receiver,
        name,
        span,
        type_arguments,
        ..
    } = expression
    else {
        unreachable!("initialization call registration requires a call occurrence")
    };
    let missing = || missing_call_target(source, *span);
    if receiver.as_deref() == Some("core") {
        return CoreOperationId::from_name(name)
            .map(Target::Core)
            .ok_or_else(missing);
    }
    if let Some(receiver) = receiver {
        let binding = match names.get(receiver) {
            Some(binding) => types.get(receiver).map(|ty| (binding.declaration_span, ty)),
            None => declarations.binding(source, receiver),
        };
        if let Some((span, ty @ ScalarType::Struct(_))) = binding {
            let field = declarations.field(ty, name).ok_or_else(missing)?;
            return Ok(Target::CallableField {
                receiver: SourceSpan::new(source.clone(), span),
                field: field.id.clone(),
            });
        }
    } else if let Some(binding) = names.get(name) {
        return Ok(Target::CallableBinding(SourceSpan::new(
            source.clone(),
            binding.declaration_span,
        )));
    }
    Ok(
        match declarations
            .callable(source, receiver.as_deref(), name)
            .ok_or_else(missing)?
        {
            CallDeclaration::Function(source, span, kind, origin) => {
                let declaration = SourceSpan::new(source.clone(), span);
                match kind {
                    CallFunctionKind::Generic => Target::Generic {
                        declaration,
                        type_arguments: type_arguments
                            .iter()
                            .map(|argument| argument.ty.clone())
                            .collect(),
                    },
                    CallFunctionKind::Ordinary => Target::Direct {
                        declaration,
                        concrete: ConcreteCallSelection::Declaration,
                    },
                    CallFunctionKind::Overload => Target::Direct {
                        declaration,
                        concrete: ConcreteCallSelection::Overload(
                            checked_overload_selection(signature, origin)
                                .cloned()
                                .ok_or_else(missing)?,
                        ),
                    },
                }
            }
            CallDeclaration::Binding(source, span) => {
                Target::CallableBinding(SourceSpan::new(source.clone(), span))
            }
            CallDeclaration::Extern(source, declaration) => {
                Target::Extern(SourceSpan::new(source.clone(), declaration.name_span))
            }
        },
    )
}

impl StructuralWalker<'_, '_> {
    fn expression_children(
        &mut self,
        expression: &ScalarExpression,
        names: &BTreeMap<String, WalkBinding>,
        types: &BTreeMap<String, ScalarType>,
        ordinal: OccurrenceOrdinal,
        initializer_statement: bool,
    ) -> Result<(), ReferenceAnalysisError> {
        let parent = OccurrenceParent::Expression(ordinal);
        match expression {
            ScalarExpression::Call { arguments, .. } => {
                if let Some(signature) = self.occurrence_typing.take_call(expression) {
                    self.checked_call_signatures.insert(ordinal, signature);
                }
                match &mut self.mode {
                    WalkMode::Callable(callable) => {
                        callable.register_call(
                            ordinal,
                            expression,
                            names,
                            types,
                            self.checked_call_signatures.get(&ordinal),
                            self.reserve_legacy_occurrences[ordinal.0],
                        )?;
                    }
                    WalkMode::CallableAllocationLength(resolver) => {
                        let (target, _) = resolver.resolve(
                            expression,
                            names,
                            types,
                            self.checked_call_signatures.get(&ordinal),
                        )?;
                        self.structural_calls.push(ordinal);
                        assert!(self
                            .allocation_length_call_targets
                            .insert(ordinal, target)
                            .is_none());
                    }
                    WalkMode::ModuleInitialization {
                        source,
                        declarations,
                    } => {
                        let target = initialization_call_target(
                            source,
                            declarations,
                            expression,
                            names,
                            types,
                            self.checked_call_signatures.get(&ordinal),
                        )?;
                        if self.reserve_legacy_occurrences[ordinal.0]
                            && matches!(
                                target,
                                ModuleInitializationCallTarget::Generic { .. }
                                    | ModuleInitializationCallTarget::Direct {
                                        concrete: ConcreteCallSelection::Overload(_),
                                        ..
                                    }
                            )
                        {
                            let unqualified =
                                matches!(expression, ScalarExpression::Call { receiver: None, .. });
                            self.initialization_reservations.push((
                                self.initialization_item,
                                ordinal,
                                unqualified,
                            ));
                        }
                        if let ModuleInitializationCallTarget::CallableField { receiver, .. } = &target {
                            self.structure.module_storage_uses.entry(receiver.clone()).or_default()
                                .push((ordinal, ModuleStorageRole::CallableReceiver));
                        }
                        self.structural_calls.push(ordinal);
                        assert!(self
                            .initialization_call_targets
                            .insert(ordinal, target)
                            .is_none());
                    }
                    WalkMode::StructuralOnly => {
                        self.structural_calls.push(ordinal);
                    }
                }
                let immediate = initializer_statement
                    && matches!(arguments.as_slice(), [ScalarExpression::Call { .. }]);
                for (position, argument) in arguments.iter().enumerate() {
                    let argument = self.expression_at(
                        argument,
                        names,
                        types,
                        parent,
                        OccurrenceEdge::CallArgument(position),
                    )?;
                    if immediate {
                        self.structure.initializer_immediate.insert(ordinal, argument);
                    }
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
                    initializer_statement,
                )?;
                let else_block = self.block(
                    else_branch,
                    names,
                    types,
                    parent,
                    OccurrenceEdge::ElseBranch,
                    initializer_statement,
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
                    initializer_statement,
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
                self.block(block, names, types, parent, OccurrenceEdge::BlockBody, false)?;
            }
            ScalarExpression::RawAddress { place, .. }
            | ScalarExpression::CheckedAddress { place, .. }
            | ScalarExpression::Dereference { place, .. }
            | ScalarExpression::IndexedRead { place, .. } => {
                let role = if matches!(expression, ScalarExpression::RawAddress { .. } | ScalarExpression::CheckedAddress { .. }) {
                    ModuleStorageRole::Address
                } else {
                    ModuleStorageRole::Project
                };
                self.place_at(place, names, types, parent, OccurrenceEdge::ExpressionPlace, role)?;
            }
            ScalarExpression::StructLiteral { fields, .. } => {
                for (position, field) in fields.iter().enumerate() {
                    let ordinal = self.expression_at(
                        &field.value,
                        names,
                        types,
                        parent,
                        OccurrenceEdge::AggregateField(position),
                    )?;
                    if let Some(id) = self.occurrence_typing.take_literal_field(&field.value) {
                        self.structure.literal_fields.insert(ordinal, id);
                    }
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
        Ok(())
    }

    fn place_at(
        &mut self,
        place: &ScalarPlace,
        names: &BTreeMap<String, WalkBinding>,
        types: &BTreeMap<String, ScalarType>,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
        storage_role: ModuleStorageRole,
    ) -> Result<OccurrenceOrdinal, ReferenceAnalysisError> {
        let ordinal = self.record_occurrence(
            OccurrenceRole::Place,
            scalar_place_target_span(place),
            parent,
            edge,
        );
        if let Some(ty) = self.occurrence_typing.take_place_type(place) {
            self.structure.checked_types.places.insert(ordinal, ty);
        }
        if edge == OccurrenceEdge::PlaceBase {
            if let Some(source) = self.occurrence_typing.take_resolved_namespace_place(place) {
                self.structure
                    .resolved_namespace_places
                    .insert(ordinal, source);
            }
        }
        let parent = OccurrenceParent::Place(ordinal);
        match place {
            ScalarPlace::Name { name, .. } => {
                if !self.structure.resolved_namespace_places.contains_key(&ordinal) {
                    let context = match &self.mode {
                        WalkMode::ModuleInitialization { source, declarations } => Some((*source, *declarations, false)),
                        WalkMode::Callable(callable) => Some((callable.target_resolver.source,
                            callable.target_resolver.declarations, true)),
                        WalkMode::CallableAllocationLength(resolver) => Some((resolver.source, resolver.declarations, true)),
                        WalkMode::StructuralOnly => None,
                    };
                    if let Some((source, declarations, callable)) = context {
                        if names.get(name).is_none_or(|binding| callable && binding.introduced_by_assignment) {
                            Self::storage_use(&mut self.structure, declarations, source, name, ordinal, storage_role);
                        }
                    }
                }
                if let WalkMode::ModuleInitialization {
                    source,
                    declarations,
                } = &self.mode
                {
                    if !matches!(edge, OccurrenceEdge::AssignmentDestination { .. }) {
                        self.occurrences[ordinal.0].requires_named_place_root = true;
                        if !self
                            .structure
                            .resolved_namespace_places
                            .contains_key(&ordinal)
                        {
                            if let Some(identity) =
                                initializer_storage(name, names, source, declarations)
                            {
                                self.initialization_identities
                                    .roots
                                    .insert(ordinal, identity);
                            }
                        }
                    }
                }
                if let NamedPlaceRootFacts::Prepared(roots) = &mut self.named_place_roots {
                    if !matches!(edge, OccurrenceEdge::AssignmentDestination { .. }) {
                        self.occurrences[ordinal.0].requires_named_place_root = true;
                        if self
                            .structure
                            .resolved_namespace_places
                            .contains_key(&ordinal)
                        {
                            return Ok(ordinal);
                        }
                        let resolver = match &self.mode {
                            WalkMode::Callable(callable) => callable.target_resolver,
                            WalkMode::CallableAllocationLength(resolver) => *resolver,
                            _ => unreachable!("named place roots are owned by a prepared callable"),
                        };
                        let module = resolver.declarations.value_token(resolver.source, name);
                        let identity = match names.get(name) {
                            Some(binding)
                                if !binding.introduced_by_assignment || module.is_none() =>
                            {
                                Some(NamedPlaceRootIdentity::Lexical(
                                    binding.reference_id(resolver.source, resolver.function_span),
                                ))
                            }
                            _ => module.map(|span| {
                                NamedPlaceRootIdentity::ModuleValue(SourceSpan::new(
                                    resolver.source.clone(),
                                    span,
                                ))
                            }),
                        };
                        if let Some(identity) = identity {
                            roots.insert(ordinal, identity);
                        }
                    }
                }
            }
            ScalarPlace::Field { base, field, .. } => {
                let base = self.place_at(base, names, types, parent, OccurrenceEdge::PlaceBase, storage_role)?;
                if let (Some(source), ScalarFieldReference::Unresolved { name, .. }) =
                    (self.structure.resolved_namespace_places.get(&base).cloned(), field)
                {
                    let declarations = match &self.mode {
                        WalkMode::ModuleInitialization { declarations, .. } => Some(*declarations),
                        WalkMode::Callable(callable) => Some(callable.target_resolver.declarations),
                        WalkMode::CallableAllocationLength(resolver) => Some(resolver.declarations),
                        WalkMode::StructuralOnly => None,
                    };
                    if let Some(declarations) = declarations {
                        Self::storage_use(&mut self.structure, declarations, &source, name, ordinal, storage_role);
                    }
                }
            }
            ScalarPlace::Index { base, index, .. } => {
                self.place_at(base, names, types, parent, OccurrenceEdge::PlaceBase, storage_role)?;
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
        outer: &BTreeMap<String, WalkBinding>,
        outer_types: &BTreeMap<String, ScalarType>,
        parent: OccurrenceParent,
        edge: OccurrenceEdge,
        initializer_statement: bool,
    ) -> Result<BlockOrdinal, ReferenceAnalysisError> {
        let ordinal = BlockOrdinal(self.structure.blocks.len());
        self.reserve_legacy_blocks
            .push(self.reserve_legacy(parent, edge));
        self.structure
            .blocks
            .push(BlockRegion::new(block.span, Vec::new()));
        assert!(self
            .structure
            .edges
            .insert((parent, edge), OccurrenceNode::Block(ordinal))
            .is_none());
        self.block_items(block, outer, outer_types, ordinal, initializer_statement)?;
        Ok(ordinal)
    }

    fn block_items(
        &mut self,
        block: &ScalarBlock,
        outer: &BTreeMap<String, WalkBinding>,
        outer_types: &BTreeMap<String, ScalarType>,
        ordinal: BlockOrdinal,
        initializer_statement: bool,
    ) -> Result<(), ReferenceAnalysisError> {
        let mut items = Vec::new();
        let mut scope = WalkScope {
            names: outer.clone(),
            types: outer_types.clone(),
        };
        for (item_index, (item, role)) in block.items_with_output_roles().enumerate() {
            items.push(self.block_item(
                item,
                role,
                BlockPosition {
                    block: ordinal,
                    item: item_index,
                },
                block.span,
                &mut scope,
                initializer_statement,
            )?);
        }
        self.structure.blocks[ordinal.0].items = items;
        Ok(())
    }

    fn binding_allocation_lengths(
        &mut self,
        binding: &ScalarBinding,
        position: BlockPosition,
        scope: &WalkScope,
    ) -> Result<BindingInitialization, ReferenceAnalysisError> {
        let callable = if let WalkMode::Callable(callable) = &self.mode {
            let length_mode = if callable.prepared {
                WalkMode::CallableAllocationLength(callable.target_resolver)
            } else {
                WalkMode::StructuralOnly
            };
            Some(std::mem::replace(&mut self.mode, length_mode))
        } else {
            None
        };
        let names = BTreeMap::new();
        let types = BTreeMap::new();
        let (names, types) = match &self.mode {
            WalkMode::ModuleInitialization { .. } | WalkMode::CallableAllocationLength(_) => {
                (&scope.names, &scope.types)
            }
            _ => (&names, &types),
        };
        let lengths = binding
            .receivers
            .iter()
            .enumerate()
            .filter_map(|(receiver, slot)| Some((receiver, slot.allocation_length.as_ref()?)))
            .map(|(receiver, length)| {
                let expression = self.expression_at(
                    length,
                    names,
                    types,
                    OccurrenceParent::Block(position.block),
                    OccurrenceEdge::BindingAllocationLength {
                        item: position.item,
                        position: receiver,
                    },
                )?;
                Ok(BindingAllocationLengthSlot {
                    position: receiver,
                    expression,
                    ty: ScalarType::U64,
                })
            })
            .collect::<Result<Vec<_>, _>>();
        if let Some(callable) = callable {
            self.mode = callable;
        }
        Ok(BindingInitialization::Allocation { lengths: lengths? })
    }

    fn binding_outputs(
        &mut self,
        binding: &ScalarBinding,
        position: BlockPosition,
        scope: &WalkScope,
    ) -> Result<BindingOutputs, ReferenceAnalysisError> {
        let parent = OccurrenceParent::Block(position.block);
        let item_index = position.item;
        let WalkScope { names, types } = scope;
        let mut evaluate = |value: &ScalarExpression, output| {
            let edge = if binding.is_allocation {
                OccurrenceEdge::QueryAllocationPlaceholder {
                    item: item_index,
                    output,
                }
            } else {
                OccurrenceEdge::BindingRoot {
                    item: item_index,
                    output,
                }
            };
            let ordinal = self.expression_at(value, names, types, parent, edge)?;
            if matches!(edge, OccurrenceEdge::QueryAllocationPlaceholder { .. })
                && matches!(value, ScalarExpression::Integer { value, span }
                    if *span == binding.span && value == &BigInt::from(0))
            {
                self.structure
                    .synthetic_expressions
                    .insert(ordinal, SyntheticOccurrence::QueryAllocationPlaceholder);
            }
            Ok::<_, ReferenceAnalysisError>(ordinal)
        };
        let ScalarBindingOutput::Evaluate(primary) = binding.output_source(0) else {
            unreachable!("primary binding output evaluates its source")
        };
        let primary = evaluate(primary, 0)?;
        let output_count = match binding.output_origin {
            ScalarBindingOutputOrigin::SingleExpression => binding.receivers.len(),
            ScalarBindingOutputOrigin::IndependentExpressions => binding.output_values.len(),
        };
        let mut secondary = Vec::with_capacity(output_count.saturating_sub(1));
        for position in 1..output_count {
            let output = match binding.output_source(position) {
                ScalarBindingOutput::Evaluate(value) => BindingOutput::Evaluate {
                    position,
                    expression: evaluate(value, position)?,
                },
                ScalarBindingOutput::ProjectPrimary { output } => {
                    BindingOutput::ProjectPrimary { output }
                }
            };
            secondary.push(output);
        }
        Ok(BindingOutputs { primary, secondary })
    }

    fn binding_region(
        &mut self,
        binding: &ScalarBinding,
        position: BlockPosition,
        block_span: ByteSpan,
        scope: &mut WalkScope,
    ) -> Result<BindingRegion, ReferenceAnalysisError> {
        let initialization = if binding.is_allocation {
            if matches!(&self.mode, WalkMode::Callable(_)) {
                let outputs = self.binding_outputs(binding, position, scope)?;
                let WalkMode::Callable(callable) = &mut self.mode else {
                    unreachable!("callable binding output traversal")
                };
                assert!(callable
                    .query_allocation_outputs
                    .insert((position.block, position.item), outputs)
                    .is_none());
            }
            self.binding_allocation_lengths(binding, position, scope)?
        } else {
            BindingInitialization::Outputs(self.binding_outputs(binding, position, scope)?)
        };
        if matches!(
            self.mode,
            WalkMode::Callable(_)
                | WalkMode::CallableAllocationLength(_)
                | WalkMode::ModuleInitialization { .. }
        ) {
            for receiver in &binding.receivers {
                scope
                    .types
                    .insert(receiver.name.clone(), receiver.ty.clone());
                scope.names.insert(
                    receiver.name.clone(),
                    WalkBinding {
                        declaration_span: receiver.name_span,
                        block_span,
                        introduced_by_assignment: false,
                    },
                );
            }
        }
        Ok(BindingRegion::new(initialization, binding))
    }

    fn block_item(
        &mut self,
        item: &ScalarBlockItem,
        role: ScalarBlockItemRole<'_>,
        position: BlockPosition,
        block_span: ByteSpan,
        scope: &mut WalkScope,
        initializer_statement: bool,
    ) -> Result<BlockItem, ReferenceAnalysisError> {
        let BlockPosition {
            block: ordinal,
            item: item_index,
        } = position;
        let parent = OccurrenceParent::Block(ordinal);
        let WalkScope { names, types } = scope;
        let kind = match item {
            ScalarBlockItem::LocalBinding(binding) => {
                BlockItemKind::Binding(self.binding_region(binding, position, block_span, scope)?)
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
                let producer = self.expression_at_with_initializer_statement(
                    expression,
                    names,
                    types,
                    parent,
                    edge,
                    initializer_statement && matches!(&role, ScalarBlockItemRole::Ordinary),
                )?;
                match role {
                    ScalarBlockItemRole::FinalOutput { output, metadata } => {
                        let mut slot = FinalOutputSlot {
                            item: item_index,
                            output,
                            expression: producer,
                            span: metadata.span,
                        };
                        if let Some(types) = self
                            .occurrence_typing
                            .take_tail_conditional_outputs(expression)
                        {
                            slot.span = expression_span(expression);
                            self.structure.blocks[ordinal.0]
                                .project_tail_outputs(slot, types.len());
                            assert!(self
                                .structure
                                .conditional_output_projections
                                .insert(producer, ConditionalOutputProjection { types })
                                .is_none());
                        }
                        BlockItemKind::FinalOutput(slot)
                    }
                    ScalarBlockItemRole::Ordinary => BlockItemKind::Evaluation {
                        expression: producer,
                    },
                }
            }
            ScalarBlockItem::Assignment(assignment) => {
                let mut evaluations = Vec::new();
                for (output, value) in assignment.rhs().evaluations() {
                    let expression = self.expression_at(
                        value,
                        names,
                        types,
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
                let mut module_outputs = match &mut self.mode {
                    WalkMode::ModuleInitialization { .. } => {
                        self.occurrence_typing.take_assignment(assignment)
                    }
                    _ => std::vec::IntoIter::default(),
                };
                let mut slots = Vec::new();
                for (destination, target) in assignment.targets.iter().enumerate() {
                    let place = self.place_at(
                        &target.place,
                        names,
                        types,
                        parent,
                        OccurrenceEdge::AssignmentDestination {
                            item: item_index,
                            destination,
                        },
                        ModuleStorageRole::Replace,
                    )?;
                    let replacement = match &self.mode {
                        WalkMode::StructuralOnly | WalkMode::CallableAllocationLength(_) => {
                            AssignmentReplacement::StructuralOnly
                        }
                        WalkMode::ModuleInitialization { .. } => module_outputs.next().map_or(
                            AssignmentReplacement::Missing,
                            AssignmentReplacement::Resolved,
                        ),
                        WalkMode::Callable(callable) => callable
                            .assignment_outputs
                            .get(&ResolvedAssignmentOutputId {
                                source: callable.target_resolver.source.clone(),
                                function_span: callable.target_resolver.function_span,
                                target_span: target.target_span,
                                output_index: destination,
                            })
                            .map_or(AssignmentReplacement::Missing, |ty| {
                                AssignmentReplacement::Resolved(ty.clone())
                            }),
                    };
                    slots.push(AssignmentSlot {
                        position: destination,
                        destination: place,
                        target_span: target.target_span,
                        replacement,
                    });
                }
                if let DirectDestinationFacts::Prepared(rows) = &mut self.direct_destinations {
                    let resolver = match &self.mode {
                        WalkMode::Callable(callable) => callable.target_resolver,
                        WalkMode::CallableAllocationLength(resolver) => *resolver,
                        _ => unreachable!("direct destinations are owned by a prepared callable"),
                    };
                    let mut introduced = BTreeMap::new();
                    for (target, slot) in assignment.targets.iter().zip(&slots) {
                        let ScalarPlace::Name { name, .. } = &target.place else {
                            continue;
                        };
                        self.occurrences[slot.destination.0].requires_direct_destination = true;
                        let identity = if let Some(binding) =
                            introduced.get(name).or_else(|| names.get(name))
                        {
                            DirectDestinationIdentity::Existing(
                                binding.reference_id(resolver.source, resolver.function_span),
                            )
                        } else if let Some(span) =
                            resolver.declarations.value_token(resolver.source, name)
                        {
                            DirectDestinationIdentity::Module(SourceSpan::new(
                                resolver.source.clone(),
                                span,
                            ))
                        } else {
                            let binding = WalkBinding {
                                declaration_span: target.target_span,
                                block_span,
                                introduced_by_assignment: true,
                            };
                            let id = binding.reference_id(resolver.source, resolver.function_span);
                            introduced.insert(name.clone(), binding);
                            DirectDestinationIdentity::Introduce {
                                binding: id,
                                name: name.clone(),
                            }
                        };
                        assert!(rows.insert(slot.destination, identity).is_none());
                    }
                    if let ScalarExpression::Name { name, .. } = &assignment.value {
                        if let Some(ty) = types.get(name).cloned() {
                            types.extend(introduced.keys().map(|name| (name.clone(), ty.clone())));
                        }
                    }
                    names.extend(introduced);
                }
                if let WalkMode::ModuleInitialization {
                    source,
                    declarations,
                } = &self.mode
                {
                    for (target, slot) in assignment.targets.iter().zip(&slots) {
                        let ScalarPlace::Name { name, .. } = &target.place else {
                            continue;
                        };
                        self.occurrences[slot.destination.0].requires_direct_destination = true;
                        let identity = if let Some(storage) =
                            initializer_storage(name, names, source, declarations)
                        {
                            InitializationDestination::Existing(storage)
                        } else if let AssignmentReplacement::Resolved(ty) = &slot.replacement {
                            if ordinal == self.structure.root {
                                InitializationDestination::IntroduceModule(target.target_span)
                            } else {
                                let binding = WalkBinding {
                                    declaration_span: target.target_span,
                                    block_span,
                                    introduced_by_assignment: true,
                                };
                                types.insert(name.clone(), ty.clone());
                                names.insert(name.clone(), binding);
                                InitializationDestination::Introduce(binding.initializer_id())
                            }
                        } else {
                            continue;
                        };
                        self.initialization_identities
                            .destinations
                            .insert(slot.destination, identity);
                    }
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
                            },
                        ),
                    )
                    .is_none());
                if matches!(&self.direct_destinations, DirectDestinationFacts::Raw)
                    && matches!(
                        &self.mode,
                        WalkMode::Callable(_) | WalkMode::CallableAllocationLength(_)
                    )
                {
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
                                    WalkBinding {
                                        declaration_span: target.target_span,
                                        block_span,
                                        introduced_by_assignment: true,
                                    },
                                );
                            }
                        }
                    }
                }
                BlockItemKind::Assignment { group: position }
            }
        };
        Ok(BlockItem {
            position: item_index,
            kind,
        })
    }
}

fn resolve_module_call_sites(
    source: &SourceIdentity,
    items: &[ScalarItem],
    namespace_bindings: &[ScalarNamespaceBinding],
    declarations: &CallDeclarationIndex<'_>,
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
    select_executable_calls: bool,
    mut occurrence_typing: PreparationOccurrenceTyping,
) -> Result<ModuleCallSites, ReferenceAnalysisError> {
    #[cfg(test)]
    stage_observations::record_call_sites();
    let mut functions = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        if let ScalarItem::Function(function) = item {
            let mut calls = resolve_function_call_sites(
                source,
                function,
                declarations,
                assignment_outputs,
                select_executable_calls,
                &mut occurrence_typing,
            )?;
            if select_executable_calls {
                calls.self_tail_candidates = self_tail::select(source, function, &calls);
            }
            functions.insert(index, calls);
        }
    }
    let mut initialization = module_initialization_region(source, items, namespace_bindings);
    let initialization_mode = if select_executable_calls {
        WalkMode::ModuleInitialization {
            source,
            declarations,
        }
    } else {
        WalkMode::StructuralOnly
    };
    initialization.detail =
        ModuleInitializationDetail::Occurrences(Box::new(walk_module_initialization(
            &initialization,
            items,
            initialization_mode,
            &mut occurrence_typing,
        )?));
    let calls = ModuleCallSites {
        functions,
        initialization,
    };
    if select_executable_calls {
        occurrence_obligations::validate_module(&calls)?;
    }
    Ok(calls)
}

fn walk_module_initialization(
    region: &ModuleInitializationRegion,
    items: &[ScalarItem],
    initialization_mode: WalkMode<'_>,
    occurrence_typing: &mut PreparationOccurrenceTyping,
) -> Result<ModuleInitializationTopology, ReferenceAnalysisError> {
    let mut walker =
        StructuralWalker::new(initialization_mode, ByteSpan::new(0, 0), occurrence_typing);
    let root = walker.structure.root;
    let mut block_items = Vec::new();
    let mut extent: Option<ByteSpan> = None;
    let mut last_install = None;
    for step in &region.steps {
        let previous_install = last_install.take();
        walker.initialization_item = step.item;
        let position = BlockPosition {
            block: root,
            item: step.item,
        };
        let span = match (&step.kind, &items[step.item]) {
            (
                ModuleInitializationStepKind::NamespaceDispatch { .. },
                ScalarItem::Namespace(namespace),
            ) => namespace.span,
            (ModuleInitializationStepKind::BindingInitialization, ScalarItem::Binding(binding)) => {
                let binding_region = walker.binding_region(
                    binding,
                    position,
                    binding.span,
                    &mut WalkScope::default(),
                )?;
                if let (WalkMode::ModuleInitialization { source, declarations },
                    BindingInitialization::Outputs(outputs), [receiver]) =
                    (&walker.mode, &binding_region.initialization, binding.receivers.as_slice())
                {
                    if outputs.secondary.is_empty() {
                        if let Some(signature) = walker.checked_call_signatures.get(&outputs.primary) {
                            if matches!(signature.outputs.outputs.as_slice(), [out] if out.ty == receiver.ty)
                                && matches!(&receiver.ty, ScalarType::Struct(id)
                                    if declarations.structs.get(id).is_some_and(|definition| definition.copy_policy == CopyPolicy::Move))
                                && matches!(walker.initialization_call_targets.get(&outputs.primary),
                                    Some(ModuleInitializationCallTarget::Direct { .. }))
                            {
                                walker.structure.module_storage_installs.insert(step.item, ModuleStorageInstall {
                                    storage: SourceSpan::new((*source).clone(), receiver.name_span),
                                    producer: outputs.primary,
                                    consumer: None,
                                });
                                last_install = Some(step.item);
                            }
                        }
                    }
                }
                block_items.push(BlockItem {
                    position: position.item,
                    kind: BlockItemKind::Binding(binding_region),
                });
                binding.span
            }
            (ModuleInitializationStepKind::ExecutableItem, ScalarItem::Executable(item)) => {
                let span = match item {
                    ScalarBlockItem::LocalBinding(binding) => binding.span,
                    ScalarBlockItem::Expression(expression) => expression_span(expression),
                    ScalarBlockItem::Assignment(assignment) => assignment.span,
                };
                let initializer_statement = matches!(&walker.mode, WalkMode::ModuleInitialization { .. });
                let node = walker.block_item(
                    item,
                    ScalarBlockItemRole::Ordinary,
                    position,
                    span,
                    &mut WalkScope::default(),
                    initializer_statement,
                )?;
                if let (Some(previous), BlockItemKind::Evaluation { expression: consumer }) =
                    (previous_install, &node.kind)
                {
                    let install = &walker.structure.module_storage_installs[&previous];
                    if let (Some(ModuleInitializationCallTarget::Direct { .. }),
                        Some(signature), Some(OccurrenceNode::Expression(argument))) =
                        (walker.initialization_call_targets.get(consumer),
                         walker.checked_call_signatures.get(consumer),
                         walker.structure.edges.get(&(OccurrenceParent::Expression(*consumer), OccurrenceEdge::CallArgument(0))))
                    {
                        let exact = matches!(signature.parameters.as_slice(), [parameter]
                            if walker.structure.checked_types.expressions.get(argument) == Some(parameter)
                            && walker.checked_call_signatures[&install.producer].outputs.outputs[0].ty == *parameter);
                        if exact && walker.structure.module_storage_uses.get(&install.storage).is_some_and(|uses|
                            uses.last() == Some(&(*argument, ModuleStorageRole::Argument { call: *consumer, slot: 0 })))
                        {
                            walker.structure.module_storage_installs.get_mut(&previous).expect("adjacent owning module storage").consumer = Some(*consumer);
                        }
                    }
                }
                block_items.push(node);
                span
            }
            _ => unreachable!("dispatch derives from this exact item slice"),
        };
        extent = Some(match extent {
            Some(previous) => {
                ByteSpan::new(previous.start.min(span.start), previous.end.max(span.end))
            }
            None => span,
        });
    }
    let root_block = &mut walker.structure.blocks[root.0];
    if let Some(span) = extent {
        root_block.span = span;
    }
    root_block.items = block_items;
    Ok(ModuleInitializationTopology {
        structure: walker.structure,
        occurrences: walker.occurrences,
        calls: walker.structural_calls,
        call_targets: walker.initialization_call_targets,
        legacy_reservations: walker.initialization_reservations,
        checked_call_signatures: walker.checked_call_signatures,
        identities: walker.initialization_identities,
        callable_escapes: walker.callable_escapes,
        assignments: walker.assignments,
        conditionals: walker.conditionals,
        short_circuits: walker.short_circuits,
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
        &CallDeclarationIndex::default(),
        assignment_outputs,
        false,
        &mut PreparationOccurrenceTyping::default(),
    )
    .expect("test callable without call declarations")
}

fn resolve_function_call_sites(
    source: &SourceIdentity,
    function: &ScalarFunction,
    declarations: &CallDeclarationIndex<'_>,
    assignment_outputs: &HashMap<ResolvedAssignmentOutputId, ScalarType>,
    prepared: bool,
    occurrence_typing: &mut PreparationOccurrenceTyping,
) -> Result<FunctionCallSites, ReferenceAnalysisError> {
    let mut arms = Vec::new();
    for arm in &function.overload_arms {
        arms.push(resolve_function_call_sites(
            source,
            arm,
            declarations,
            assignment_outputs,
            prepared,
            occurrence_typing,
        )?);
    }
    let callable = CallableWalk {
        target_resolver: CallableTargetResolver {
            source,
            function_span: function.span,
            declarations,
        },
        prepared,
        assignment_outputs,
        sites: Vec::new(),
        site_by_span: BTreeMap::new(),
        call_occurrences: Vec::new(),
        declared_unsafe_calls: BTreeSet::new(),
        overload_reservation_sites: Vec::new(),
        query_allocation_outputs: HashMap::new(),
    };
    let mut resolver = StructuralWalker::new(
        WalkMode::Callable(callable),
        function.body.span,
        occurrence_typing,
    );
    let mut parameter_names = BTreeMap::new();
    let mut parameter_types = BTreeMap::new();
    let ScalarType::Callable {
        parameters,
        outputs,
    } = &function.signature
    else {
        unreachable!("validated function signature")
    };
    let parameters = function
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
        .collect::<Vec<_>>();
    for slot in &parameters {
        parameter_types.insert(slot.name.clone(), slot.ty.clone());
        parameter_names.insert(
            slot.name.clone(),
            WalkBinding {
                declaration_span: slot.span,
                block_span: function.body.span,
                introduced_by_assignment: false,
            },
        );
    }
    let root = resolver.structure.root;
    resolver.block_items(&function.body, &parameter_names, &parameter_types, root, false)?;
    let root_conditional = resolver.structure.blocks[root.0]
        .items
        .last()
        .and_then(|item| match &item.kind {
            BlockItemKind::FinalOutput(slot) => resolver
                .structure
                .conditional_output_projections
                .get(&slot.expression),
            _ => None,
        });
    let root_output = |(output, ty): (usize, &ScalarType)| RootOutputType {
        output,
        ty: ty.clone(),
    };
    let root_outputs = match root_conditional {
        Some(row) => row.types.iter().enumerate().map(root_output).collect(),
        None => function
            .body
            .final_output_values
            .iter()
            .map(|value| &value.ty)
            .enumerate()
            .map(root_output)
            .collect(),
    };
    let boundary = CallableBoundary {
        parameters,
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
        root_outputs,
        generics: function.generic_parameters.clone(),
    };
    let WalkMode::Callable(callable) = resolver.mode else {
        unreachable!("callable structural traversal")
    };
    Ok(FunctionCallSites {
        boundary,
        sites: callable.sites,
        arms,
        occurrences: resolver.occurrences,
        call_occurrences: callable.call_occurrences,
        declared_unsafe_calls: callable.declared_unsafe_calls,
        callable_escapes: resolver.callable_escapes,
        overload_reservation_sites: callable.overload_reservation_sites,
        self_tail_candidates: Vec::new(),
        allocation_length_calls: resolver.structural_calls,
        allocation_length_call_targets: resolver.allocation_length_call_targets,
        checked_call_signatures: resolver.checked_call_signatures,
        plain_names: resolver.plain_names,
        named_place_roots: resolver.named_place_roots,
        direct_destinations: resolver.direct_destinations,
        members: resolver.members,
        query_allocation_outputs: callable.query_allocation_outputs,
        structure: resolver.structure,
        assignments: resolver.assignments,
        conditionals: resolver.conditionals,
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
        derive_module_reference_origins(source, items, call_sites, structs, enums, false);
    install_module_reference_results(items, updates);
    errors
}

fn derive_module_reference_origins(
    source: &SourceIdentity,
    items: &[ScalarItem],
    call_sites: &ModuleCallSites,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    capture_call_points: bool,
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
                    capture_call_points,
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
        Some(
            &self
                .function_result(module_ordinal, item_ordinal, arm_index)?
                .cfg,
        )
    }

    pub(super) fn callable_point_facts(
        &self,
        module: usize,
        item: usize,
        arm: Option<usize>,
    ) -> Option<&CallablePointFacts> {
        Some(&self.function_result(module, item, arm)?.proof_points)
    }

    fn function_result(
        &self,
        module_ordinal: usize,
        item_ordinal: usize,
        arm_index: Option<usize>,
    ) -> Option<&FunctionReferenceResult> {
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
        Some(body)
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
    analyze_project_references_with_call_points(modules, call_sites, false)
}

pub(super) fn analyze_project_references_with_call_points(
    modules: &[ScalarModule],
    call_sites: &ProjectCallSiteResults,
    capture_call_points: bool,
) -> ProjectReferenceResults {
    let (mut updates, mut errors, inventory) =
        derive_and_converge_project_references(modules, call_sites, capture_call_points);
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
    capture_call_points: bool,
) -> (
    Vec<(usize, Vec<(usize, FunctionReferenceResult)>)>,
    Vec<ReferenceAnalysisError>,
    Option<(Vec<ScalarStruct>, Vec<ScalarEnum>)>,
) {
    let (mut updates, mut errors, inventory) =
        derive_project_reference_origins(modules, call_sites, capture_call_points);
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
    capture_call_points: bool,
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
                    capture_call_points,
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ReferenceCallableBody {
    Declaration,
    Overload(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReferenceOutputContract<'a> {
    source: &'a SourceIdentity,
    declaration_span: ByteSpan,
    body: ReferenceCallableBody,
    output: usize,
    origins: Vec<ReferenceOrigin>,
    independent_static: bool,
}

struct ReferenceOutputContractMatch {
    first: usize,
    unique: bool,
}

type ReferenceContractKey = (ByteSpan, ReferenceCallableBody, usize);

#[derive(Default)]
struct ReferenceOutputContracts<'a> {
    rows: Vec<ReferenceOutputContract<'a>>,
    by_source: HashMap<&'a SourceIdentity, HashMap<ReferenceContractKey, ReferenceOutputContractMatch>>,
}

impl<'a> ReferenceOutputContracts<'a> {
    fn new(rows: Vec<ReferenceOutputContract<'a>>) -> Self {
        let mut by_source = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            by_source.entry(row.source).or_insert_with(HashMap::new)
                .entry((row.declaration_span, row.body, row.output))
                .and_modify(|found: &mut ReferenceOutputContractMatch| found.unique = false)
                .or_insert(ReferenceOutputContractMatch { first: index, unique: true });
        }
        Self { rows, by_source }
    }

    fn find(
        &self,
        source: &SourceIdentity,
        declaration_span: ByteSpan,
        selection: &ConcreteCallSelection,
        output: usize,
    ) -> Option<(&ReferenceOutputContract<'a>, bool)> {
        let body = match selection {
            ConcreteCallSelection::Declaration => ReferenceCallableBody::Declaration,
            ConcreteCallSelection::Overload(arm) => ReferenceCallableBody::Overload(arm.arm_index),
        };
        let found = self.by_source.get(source)?.get(&(declaration_span, body, output))?;
        Some((&self.rows[found.first], found.unique))
    }
}

#[derive(Default)]
struct ReferenceContractReplay {
    module_calls: Vec<CfgPointId>,
    errors: Vec<ReferenceAnalysisError>,
}

fn collect_function_output_contracts<'a>(
    source: &'a SourceIdentity,
    result: &mut FunctionReferenceResult,
    declaration_span: ByteSpan,
    body: ReferenceCallableBody,
    contracts: &mut Vec<ReferenceOutputContract<'a>>,
) {
    let mut independent = !result.aggregate.checked_result_origins.is_empty();
    let mut saw_checked = false;
    let mut checked_successor_result = result.aggregate.checked_successor_contract;
    let mut raw_read_forward_result = result.aggregate.raw_read_forward_contract;
    let start = contracts.len();
    for &CheckedContractReturn { point, output, value_point } in &result.checked_contract_returns {
        let point = &result.cfg.points[point.0];
        if checked_successor_result || raw_read_forward_result {
            let exact_forward = !saw_checked && output == 0
                && point.possible_origins.as_ref().is_some_and(|origins| !origins.is_empty()
                    && result.cfg.points[value_point.0].possible_origins.as_ref() == Some(origins));
            checked_successor_result &= exact_forward
                && result.aggregate.checked_successor_candidate == Some(value_point);
            raw_read_forward_result &= exact_forward
                && result.aggregate.raw_read_forward_candidate == Some(value_point);
        }
        saw_checked = true;
        let Some(origins) = &point.possible_origins else {
            independent = false;
            continue;
        };
        independent = independent && output == 0 && !origins.is_empty();
        contracts.push(ReferenceOutputContract {
            source,
            declaration_span,
            body,
            output,
            origins: origins
                .iter()
                .map(|id| {
                    independent = independent && result.aggregate.checked_result_origins.contains(id);
                    result.cfg.facts[id.0].clone()
                })
                .collect(),
            independent_static: false,
        });
    }
    result.aggregate.independent_checked_result = saw_checked && independent;
    result.aggregate.checked_successor_result = saw_checked && checked_successor_result;
    result.aggregate.raw_read_forward_result = saw_checked && raw_read_forward_result;
    if result.aggregate.independent_checked_result && contracts.len() == start + 1 {
        contracts[start].independent_static = true;
    }
    for (index, arm) in result.arms.iter_mut().enumerate() {
        collect_function_output_contracts(
            source,
            arm,
            declaration_span,
            ReferenceCallableBody::Overload(index),
            contracts,
        );
    }
}

fn transfer_function_output_contracts<'a>(
    result: &mut FunctionReferenceResult,
    previous: &ReferenceOutputContracts<'_>,
    contracts: &'a ReferenceOutputContracts<'_>,
    inputs: &mut HashMap<CfgPointId, &'a [ReferenceOrigin]>,
    replays: &mut Vec<ReferenceContractReplay>,
    cursor: &mut usize,
) {
    for arm in &mut result.arms {
        transfer_function_output_contracts(arm, previous, contracts, inputs, replays, cursor);
    }
    let index = *cursor;
    *cursor += 1;
    let initial = index == replays.len();
    if initial {
        replays.push(ReferenceContractReplay {
            module_calls: result.cfg.initial_points.iter().filter_map(|point| {
                matches!(&point.kind, CfgPointKind::Call {
                    expression: ScalarExpression::Call { .. },
                    target: ResolvedCallTarget::ModuleCallable { .. },
                    ..
                }).then_some(point.id)
            }).collect(),
            ..ReferenceContractReplay::default()
        });
    }
    let replay = &mut replays[index];
    let mut changed = initial;
    inputs.clear();
    result.aggregate.checked_successor_contract = false;
    result.aggregate.raw_read_forward_contract = false;
    for &id in &replay.module_calls {
        let CfgPointKind::Call {
            expression: ScalarExpression::Call { type_arguments, .. },
            output,
            target: ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete },
            ..
        } = &result.cfg.initial_points[id.0].kind else {
            unreachable!("recorded module call point")
        };
        let current = contracts.find(source, *declaration_span, concrete, *output);
        if !changed {
            let prior = previous.find(source, *declaration_span, concrete, *output);
            changed = current.map(|(row, _)| &row.origins) != prior.map(|(row, _)| &row.origins);
        }
        if let Some((row, unique)) = current {
            inputs.insert(id, row.origins.as_slice());
            if *output == 0 && type_arguments.is_empty() && row.independent_static && unique {
                result.aggregate.checked_successor_contract |= result.aggregate.checked_successor_candidate == Some(id);
                result.aggregate.raw_read_forward_contract |= result.aggregate.raw_read_forward_candidate == Some(id)
                    && match concrete {
                        ConcreteCallSelection::Declaration => true,
                        ConcreteCallSelection::Overload(arm) => arm.substitutions.is_empty(),
                    };
            }
        }
    }
    if !result.cfg.initial_points.is_empty() && changed {
        result.cfg.points = result.cfg.initial_points.clone();
        let candidate = match (&result.proof_points.scalar_descriptor_install, &result.proof_points.backing_descriptor_install) {
            (ScalarDescriptorCandidates::One(destination, marker), _) => Some((destination, marker)),
            (ScalarDescriptorCandidates::None, ScalarDescriptorCandidates::One(destination, marker))
                if !result.proof_points.has_nonlexical_install && result.cfg.allocations.is_empty()
                => Some((destination, marker)),
            _ => None,
        };
        let mut descriptor = candidate.and_then(|(_, marker)| {
            let capture = result.cfg.points.get(marker.capture.0)?;
            matches!(&capture.kind, CfgPointKind::CaptureAssignmentPrevious { writes }
                if writes.get(marker.output) == Some(&marker.frozen.install))
                .then_some(ScalarDescriptorReplay { candidate: marker.as_ref(), passive: result.aggregate.passive_descriptor_read.as_deref().filter(|_| marker.validate_backing),
                    invalid: false, exited: false })
        });
        let transferred = transfer_reference_facts(
            ReferenceTransferCfg::new(&mut result.cfg, Some(&result.proof_points)),
            &BTreeMap::new(),
            inputs,
            None,
            descriptor.as_mut(),
        );
        result.aggregate.checked_producer_authoritative = transferred.is_empty()
            && !result.aggregate.checked_producer_sources.is_empty()
            && result.aggregate.checked_producer_sources.iter().all(|(_, terminal)| {
                use aggregate::CheckedProducerTerminal;
                let valid = |id: CfgPointId, matches: &dyn Fn(&ReferenceOrigin) -> bool| {
                    result.cfg.points[id.0].possible_origins.as_ref().is_some_and(|origins| {
                        origins.len() == 1 && origins.iter().all(|origin| matches(&result.cfg.facts[origin.0]))
                    })
                };
                match terminal {
                    CheckedProducerTerminal::Null(point) => valid(*point, &|origin| matches!(origin, ReferenceOrigin::Null { .. })),
                    CheckedProducerTerminal::Fresh { borrow, descriptor } => {
                        valid(*borrow, &|origin| matches!(origin, ReferenceOrigin::Fresh { loan, allocation }
                            if loan.origin_place.binding == *descriptor && allocation.source == descriptor.source
                                && allocation.function_span == descriptor.function_span))
                    }
                    CheckedProducerTerminal::Unknown => true,
                }
            });
        result.aggregate.checked_backing_forward = result.aggregate.checked_backing_forward_candidate
            .filter(|&(_, primary, terminal)| {
                let Some(origins) = result.cfg.points[primary.0].possible_origins.as_ref() else { return false };
                transferred.is_empty()
                    && result.cfg.points[terminal.0].possible_origins.as_ref() == Some(origins)
                    && origins.iter().any(|id| matches!(&result.cfg.facts[id.0], ReferenceOrigin::Fresh { loan, .. }
                        if matches!(loan.origin_place.binding.kind, ReferenceBindingKind::CallOutput { point, output: 0 } if point == primary)))
                    && origins.iter().all(|id| matches!(&result.cfg.facts[id.0], ReferenceOrigin::Null { .. })
                        || matches!(&result.cfg.facts[id.0], ReferenceOrigin::Fresh { loan, .. }
                            if matches!(loan.origin_place.binding.kind, ReferenceBindingKind::CallOutput { point, output: 0 } if point == primary)))
            }).map(|(ordinal, primary, _)| (ordinal, primary));
        let install = descriptor.filter(|proof| transferred.is_empty() && proof.exited && !proof.invalid)
            .and_then(|proof| candidate.map(|(destination, _)| (*destination, proof.candidate)));
        result.aggregate.scalar_descriptor_assignment = install
            .filter(|(_, marker)| !marker.validate_backing)
            .map(|(destination, marker)| (destination.0, marker.call.0, marker.output));
        result.aggregate.checked_backing_assignment = install
            .filter(|(_, marker)| marker.validate_backing)
            .map(|(destination, marker)| (destination.0, marker.call.0, marker.frozen.rhs));
        replay.errors = transferred;
    }
}

fn resolve_reference_output_contracts(
    modules: &mut [ModuleReferenceView<'_>],
) -> Vec<ReferenceAnalysisError> {
    #[cfg(test)]
    stage_observations::record_output_contracts();
    let mut previous = ReferenceOutputContracts::default();
    let mut contracts = ReferenceOutputContracts::default();
    let mut replays = Vec::new();
    loop {
        let mut inputs = HashMap::new();
        let mut cursor = 0;
        for module in modules.iter_mut() {
            for (_, result) in module.results.iter_mut() {
                transfer_function_output_contracts(
                    result, &previous, &contracts, &mut inputs, &mut replays, &mut cursor,
                );
            }
        }
        drop(inputs);
        let mut next = Vec::new();
        for module in modules.iter_mut() {
            for (index, result) in module.results.iter_mut() {
                let ScalarItem::Function(function) = &module.items[*index] else {
                    unreachable!("function result ordinal")
                };
                collect_function_output_contracts(
                    module.source,
                    result,
                    function.name_span,
                    ReferenceCallableBody::Declaration,
                    &mut next,
                );
            }
        }
        if next == contracts.rows {
            return replays.into_iter().flat_map(|replay| replay.errors).collect();
        }
        previous = std::mem::replace(&mut contracts, ReferenceOutputContracts::new(next));
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
            let template_cfg = &template_result.cfg;
            let mut points = template_cfg.initial_points.clone();
            let mut facts = template_cfg.facts.clone();
            let transferred = transfer_reference_facts(
                ReferenceTransferCfg {
                    initial_points: &template_cfg.initial_points,
                    pending_named_reborrow: template_result.proof_points.pending_named_reborrow.as_ref(),
                    resolve_pending_named_reborrow: false,
                    raw_address_exposures: &template_result.proof_points.raw_address_exposures,
                    points: &mut points,
                    facts: &mut facts,
                    allocations: &template_cfg.allocations,
                    checked_join_values: &template_result.proof_points.checked_join_values,
                    checked_block_values: &template_result.proof_points.checked_block_values,
                    checked_binding_initializers: &template_result.proof_points.checked_binding_initializers,
                    resolved_checked_call_outputs: &template_result.proof_points.resolved_checked_call_outputs,
                    checked_pointee_reads: &template_result.proof_points.checked_pointee_reads,
                    deferred_child_scalar_stores: &template_result.proof_points.deferred_child_scalar_stores,
                    named_reborrow_parents: &template_result.proof_points.named_reborrow_parents,
                },
                &deferred,
                &HashMap::new(),
                None,
                None,
            );
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

struct CheckedContractReturn {
    point: CfgPointId,
    output: usize,
    value_point: CfgPointId,
}

impl CheckedContractReturn {
    fn collect(signature: &ScalarType, points: &[ReferenceFlowPoint]) -> Vec<Self> {
        let ScalarType::Callable { outputs, .. } = signature else {
            return Vec::new();
        };
        if !outputs.outputs.iter().any(|output| matches!(output.ty, ScalarType::CheckedReference { .. })) {
            return Vec::new();
        }
        points.iter().filter_map(|point| {
            let CfgPointKind::ReturnOutput { output, value_point, .. } = point.kind else {
                return None;
            };
            matches!(outputs.outputs[output].ty, ScalarType::CheckedReference { .. }).then_some(Self {
                point: point.id,
                output,
                value_point,
            })
        }).collect()
    }
}

struct FunctionReferenceResult {
    cfg: ScalarReferenceCfg,
    checked_contract_returns: Vec<CheckedContractReturn>,
    proof_points: CallablePointFacts,
    aggregate: aggregate::AggregateFlowResult,
    arms: Vec<FunctionReferenceResult>,
}

impl FunctionReferenceResult {
    fn install(self, function: &mut ScalarFunction) {
        self.install_body(function, 0, None, None);
    }

    fn install_body(self, function: &mut ScalarFunction, item: usize, arm: Option<usize>, mut facts: Option<&mut aggregate::InstalledAggregateFacts>) {
        assert_eq!(self.arms.len(), function.overload_arms.len());
        for (index, (result, arm)) in self.arms.into_iter().zip(&mut function.overload_arms).enumerate() {
            result.install_body(arm, item, Some(index), facts.as_deref_mut());
        }
        function.reference_cfg = self.cfg;
        if let Some(facts) = facts {
            if self.aggregate.has_observations() {
                facts.insert((item, arm), self.aggregate);
            }
        }
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
        false,
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
    capture_call_points: bool,
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
            capture_call_points,
            errors,
        ));
    }
    let mut builder =
        ReferenceFlowBuilder::new(source, function, module_names, call_sites, structs, enums);
    builder.capture_call_points = capture_call_points;
    let (cfg, current_errors, proof_points, aggregate) =
        build_function_reference_cfg(builder, function, errors.len() == entry_errors);
    errors.extend(current_errors);
    FunctionReferenceResult {
        checked_contract_returns: CheckedContractReturn::collect(&function.signature, &cfg.initial_points),
        cfg,
        arms,
        proof_points,
        aggregate,
    }
}

#[cfg(test)]
fn derive_function_reference_cfg(
    source: &SourceIdentity,
    function: &ScalarFunction,
    module_names: &HashSet<String>,
    call_sites: &FunctionCallSites,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    prior_children_succeeded: bool,
) -> (ScalarReferenceCfg, Vec<ReferenceAnalysisError>) {
    let builder =
        ReferenceFlowBuilder::new(source, function, module_names, call_sites, structs, enums);
    let (cfg, errors, _, _) =
        build_function_reference_cfg(builder, function, prior_children_succeeded);
    (cfg, errors)
}

fn build_function_reference_cfg(
    mut builder: ReferenceFlowBuilder<'_>,
    function: &ScalarFunction,
    prior_children_succeeded: bool,
) -> (
    ScalarReferenceCfg,
    Vec<ReferenceAnalysisError>,
    CallablePointFacts,
    aggregate::AggregateFlowResult,
) {
    let mut errors = Vec::new();
    let call_sites = builder.call_sites;
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
                source: builder.source.clone(),
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
        if builder.capture_call_points {
            builder
                .proof_points
                .formals
                .insert(slot.ordinal, builder.names[name].clone());
        }
        builder
            .binding_types
            .insert(builder.names[name].clone(), ty.clone());
        if slot.ordinal == 0 && matches!(&call_sites.named_place_roots, NamedPlaceRootFacts::Prepared(_))
            && matches!(&function.signature, ScalarType::Callable { parameters, outputs }
            if aggregate::array_roundtrip_signature(parameters, outputs))
        {
            builder.aggregate_facts.array_formal = Some(builder.names[name].clone());
            builder.aggregate_result.array_neutral = matches!(&function.signature, ScalarType::Callable {
                parameters, outputs
            } if aggregate::neutral_array_signature(parameters, outputs))
                && call_sites.allocation_length_call_targets.is_empty()
                && call_sites.callable_escapes.is_empty()
                && call_sites.structure.module_storage_uses.is_empty();
        }
        if builder.aggregate_facts.array_formal.is_some() && aggregate::array_neutral_scalar(ty) {
            builder.aggregate_facts.neutral_array_scalars.insert(builder.names[name].clone());
        }
        if let ScalarType::Struct(id) = ty {
            builder.aggregate_facts.formals.insert(slot.ordinal, builder.names[name].clone());
            if let Some(declaration) = builder.structs.iter().find(|structure| &structure.id == id) {
                let mut raw = declaration.fields.iter().filter(|field| matches!(field.ty, ScalarType::RawPointer(_)));
                if let (Some(only), None) = (raw.next(), raw.next()) {
                    builder.aggregate_facts.read_fields.insert(slot.ordinal, only.id.clone());
                }
            }
        }
        if matches!(ty, ScalarType::RawPointer(_)) {
            builder.aggregate_facts.raw_formals.insert(slot.ordinal, builder.names[name].clone());
        }
        if let ScalarType::CheckedReference { mutability, .. } = ty {
            let binding = builder.names[name].clone();
            let loan = ReferenceLoanId {
                mode: *mutability,
                origin_place: ReferencePlaceId {
                    source: builder.source.clone(),
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
            let formal_parent = matches!(ty, ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Mutable, inner,
            } if aggregate::ordinary_value(inner))
                && deferred_named_reborrow_allowed(function, call_sites)
                && call_sites.boundary.parameters.iter().all(|parameter|
                    parameter.ordinal == slot.ordinal || aggregate::ordinary_value(&parameter.ty));
            let formal = formal_parent.then(|| binding.clone());
            let entry = builder.point(
                CfgPointKind::Bind { binding },
                *span,
                ReferenceScopeId(0),
                BTreeSet::from([id]),
            );
            if let Some(formal) = formal {
                builder.deferred_reborrow_roots.push((
                    formal, super::control::DeferredReborrowRoot::Formal { parameter: slot.ordinal }, entry, id,
                ));
            }
        }
    }
    if builder.aggregate_facts.formals.len() == 1 && builder.aggregate_facts.raw_formals.is_empty()
        && (aggregate::ordinary_formal_outputs(&call_sites.boundary.root_outputs)
            || builder.independent_checked_query)
    {
        builder.formal_forward_shape = Some((None, None));
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
    drop(builder.origin_index);
    let mut aggregate = std::mem::take(&mut builder.aggregate_result);
    if let ScalarDescriptorCandidates::One(_, marker) = &builder.proof_points.backing_descriptor_install {
        aggregate.passive_descriptor_read = passive_descriptor_read(
            &builder.cfg, marker, &builder.aggregate_facts, call_sites,
            &builder.proof_points, builder.structs).map(Box::new);
    }
    if prior_children_succeeded && errors.is_empty() && !builder.cfg.points.is_empty() {
        builder.cfg.initial_points = builder.cfg.points.clone();
        if let Some((Some(moved), Some(call))) = builder.formal_forward_shape {
            let terminal = builder.cfg.points.last().map(|point| &point.kind);
            if matches!(terminal, Some(CfgPointKind::Return))
                || (call_sites.boundary.root_outputs.is_empty()
                    && matches!(terminal, Some(CfgPointKind::ScopeExit)))
            {
                builder.aggregate_facts.formal_forward_shape = Some((moved, call));
            }
        }
        let flow = builder.aggregate_facts.observes_values().then_some((
            &builder.aggregate_facts,
            call_sites,
            &mut aggregate,
        ));
        let mut provisional_cfg = ReferenceTransferCfg::new(&mut builder.cfg, Some(&builder.proof_points));
        provisional_cfg.resolve_pending_named_reborrow = true;
        let provisional = transfer_reference_facts(
            provisional_cfg,
            &BTreeMap::new(),
            &HashMap::new(),
            flow,
            None,
        );
        if let Some(pending) = &builder.proof_points.pending_named_reborrow {
            let first_unproved = pending.sites.iter().find(|site|
                !matches!(builder.cfg.points[site.point.0].kind, CfgPointKind::Borrow { .. }));
            if first_unproved.is_none() && provisional.is_empty() {
                for site in &pending.sites {
                    builder.cfg.initial_points[site.point.0] = builder.cfg.points[site.point.0].clone();
                }
            } else {
                let prior_errors = errors.len();
                if let super::control::DeferredReborrowRoot::Local { borrow, .. } = &pending.root {
                    let root = &builder.cfg.initial_points[borrow.0].source_span;
                    errors.extend(provisional.iter().filter(|error| matches!(error,
                        ReferenceAnalysisError::ConflictingPlace { source_span, origins }
                            if (origins.contains(root) || pending.sites.iter().any(|site| {
                                let solved = &builder.cfg.points[site.point.0];
                                matches!(solved.kind, CfgPointKind::Borrow { .. }) && origins.contains(&solved.source_span)
                            }))
                                && builder.cfg.initial_points.iter().any(|point| point.source_span == *source_span
                                    && deferred_local_owner_store(pending, point, &builder.cfg.initial_points))
                    )).cloned());
                }
                if pending.sites.len() > 1 || is_pending_shared_reborrow(&builder.cfg.initial_points[pending.sites[0].point.0]) {
                    errors.extend(provisional.iter().filter(|error| matches!(error,
                        ReferenceAnalysisError::ConflictingPlace { source_span, origins } if pending.sites.iter().any(|site| {
                            let solved = &builder.cfg.points[site.point.0];
                            let CfgPointKind::Borrow { loan, .. } = &solved.kind else { return false };
                            solved.source_span == *source_span
                                || ((site.parent.is_some() || loan.mode == ScalarReferenceMutability::Shared)
                                        && origins.contains(&solved.source_span)
                                            && builder.proof_points.checked_pointee_reads.iter()
                                                .chain(&builder.proof_points.deferred_child_scalar_stores)
                                                .any(|access| {
                                                    let point = &builder.cfg.points[access.0];
                                                    let pointer = match &point.kind {
                                                        CfgPointKind::ReadPlace { pointer_point: Some(pointer), .. }
                                                        | CfgPointKind::AssignThrough { pointer_point: pointer, .. } => *pointer,
                                                        _ => return false,
                                                    };
                                                    point.source_span == *source_span
                                                        && matches!(&builder.cfg.initial_points[pointer.0].kind,
                                                            CfgPointKind::Read { binding }
                                                                if binding == &pending.handle || site.parent.is_some_and(|(_, holder)| matches!(
                                                                    &builder.cfg.initial_points[holder.0].kind,
                                                                    CfgPointKind::Bind { binding: parent } if parent == binding)))
                                                }))
                        })
                    )).cloned());
                }
                if errors.len() == prior_errors {
                    let refused = first_unproved.unwrap_or(&pending.sites[0]);
                    errors.push(ReferenceAnalysisError::MissingOrigin {
                        source_span: builder.cfg.points[refused.point.0].source_span.clone(),
                        binding_id: pending.handle.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    });
                }
            }
        }
        if provisional.is_empty() {
            for &(end, _) in &builder.aggregate_facts.checked_producer_arms {
                aggregate.checked_producer_backing.entry(end).or_insert(aggregate::CheckedProducerBacking::Unknown);
            }
            aggregate.checked_producer_sources = std::mem::take(&mut builder.aggregate_facts.checked_producer_arms);
        } else {
            aggregate.checked_producer_backing.clear();
        }
    }
    aggregate.returning_leaf = errors.is_empty() && prior_children_succeeded && builder.returning_leaf_prefix
        && matches!(builder.cfg.points.last().map(|point| &point.kind), Some(CfgPointKind::Return));
    if builder.independent_checked_query {
        aggregate.checked_successor_candidate = aggregate.formal_forward.map(|(_, point, _, _)| point);
        if !builder.cfg.initial_points.is_empty()
            && (!builder.aggregate_facts.raw_formals.is_empty()
                || !builder.aggregate_facts.read_fields.is_empty())
        {
            let returned = builder.cfg.initial_points.iter().rev().find_map(|point| {
                match point.kind {
                    CfgPointKind::ReturnOutput { output: 0, value_point, checked: true } => Some(value_point),
                    _ => None,
                }
            });
            aggregate.raw_read_forward_candidate = returned.filter(|point| {
                matches!(&builder.cfg.initial_points[point.0].kind,
                    CfgPointKind::Call { output: 0, target: ResolvedCallTarget::ModuleCallable { .. }, .. })
            });
        }
    }
    if !errors.is_empty() || !prior_children_succeeded {
        aggregate.array_returns.clear();
        aggregate.array_rebinds.clear();
        aggregate.local_array_failure_arms.clear();
        aggregate.checked_producer_backing.clear();
    }
    aggregate.array_calls = builder.aggregate_facts.array_calls;
    aggregate.checked_result_origins = builder.checked_result_origins;
    aggregate.allocation_slots = builder.aggregate_facts.allocation_slots;
    aggregate.calls = builder.aggregate_facts.calls;
    aggregate.formals = builder.aggregate_facts.formals;
    aggregate.read_formals = builder.aggregate_facts.raw_formals.into_keys().collect();
    aggregate.read_formal_fields = builder.aggregate_facts.read_fields;
    (builder.cfg, errors, builder.proof_points, aggregate)
}

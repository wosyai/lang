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
pub(super) mod local_borrow;

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

struct ReferenceBindingType {
    ty: ScalarType,
    // Present only for a local declaration/assignment introduction in a
    // prepared body eligible for local deferred reborrowing.
    local_reborrow_scope: Option<ReferenceScopeId>,
}

#[derive(Default)]
pub(super) struct ReferenceBindingTypes(HashMap<ReferenceBindingId, ReferenceBindingType>);

impl ReferenceBindingTypes {
    fn get(&self, binding: &ReferenceBindingId) -> Option<&ScalarType> {
        self.0.get(binding).map(|entry| &entry.ty)
    }

    fn contains_key(&self, binding: &ReferenceBindingId) -> bool {
        self.0.contains_key(binding)
    }

    fn record(&mut self, binding: ReferenceBindingId, ty: ScalarType, local_reborrow_scope: Option<ReferenceScopeId>) {
        self.0.insert(binding, ReferenceBindingType { ty, local_reborrow_scope });
    }

    #[cfg(test)]
    fn insert(&mut self, binding: ReferenceBindingId, ty: ScalarType) {
        self.record(binding, ty, None);
    }

    fn remove(&mut self, binding: &ReferenceBindingId) {
        self.0.remove(binding);
    }

    fn is_local_reborrow_owner(
        &self, binding: &ReferenceBindingId, borrow: ReferenceScopeId, destination: ReferenceScopeId,
    ) -> bool {
        borrow == destination && self.0.get(binding).is_some_and(|entry|
            entry.local_reborrow_scope == Some(borrow) && aggregate::ordinary_value(&entry.ty))
    }
}

#[cfg(test)]
#[test]
fn prepared_direct_local_reborrow_owner_requires_the_exact_declared_scope() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let binding = |start| ReferenceBindingId {
        source: source.clone(), function_span: span, block_span: span,
        declaration_span: ByteSpan::new(start, start + 1), kind: ReferenceBindingKind::Declared,
    };
    let local = binding(1);
    let same_block_span = binding(2);
    let formal = binding(3);
    let mut types = ReferenceBindingTypes::default();
    let scope = ReferenceScopeId(1);
    let other_scope = ReferenceScopeId(2);
    types.record(local.clone(), ScalarType::U64, Some(scope));
    types.record(same_block_span.clone(), ScalarType::U64, Some(other_scope));
    types.record(formal.clone(), ScalarType::U64, None);
    assert_eq!(types.get(&local), Some(&ScalarType::U64));
    assert!(types.is_local_reborrow_owner(&local, scope, scope));
    assert!(!types.is_local_reborrow_owner(&local, scope, other_scope));
    assert!(!types.is_local_reborrow_owner(&same_block_span, scope, scope));
    assert!(!types.is_local_reborrow_owner(&formal, scope, scope));
    types.remove(&local);
    assert!(!types.is_local_reborrow_owner(&local, scope, scope));
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GuardedRuntimeMove {
    pub(crate) source: ReferenceBindingId,
    pub(crate) target: ReferenceBindingId,
    pub(crate) target_ty: ScalarType,
    pub(crate) rhs: usize,
    pub(crate) replacement: Option<usize>,
    moved: CfgPointId,
    scope: ReferenceScopeId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GuardedScalarAllocation {
    binding: ReferenceBindingId,
    ty: ScalarType,
    runtime_literal_length: Option<u64>,
    runtime_returned: bool,
    guards: Box<[GuardedScalarBranch]>,
    alias_accesses: Box<[usize]>,
    runtime_moves: Box<[GuardedRuntimeMove]>,
    releases: Box<[ByteSpan]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GuardedScalarBranch {
    pub(crate) condition: usize,
    pub(crate) branch: CfgPointId,
    pub(crate) present_on_then: bool,
}

impl GuardedScalarAllocation {
    pub(crate) fn binding(&self) -> &ReferenceBindingId { &self.binding }
    pub(crate) fn ty(&self) -> &ScalarType { &self.ty }
    pub(crate) fn runtime_moves(&self) -> &[GuardedRuntimeMove] { &self.runtime_moves }
    pub(crate) fn releases(&self) -> &[ByteSpan] { &self.releases }
    pub(crate) fn runtime_returned(&self) -> bool { self.runtime_returned }
    pub(crate) fn runtime_literal_length(&self) -> Option<u64> { self.runtime_literal_length }
    #[cfg(test)]
    pub(crate) fn condition(&self) -> usize { self.guards[0].condition }
    #[cfg(test)]
    pub(crate) fn branch(&self) -> CfgPointId { self.guards[0].branch }
    #[cfg(test)]
    pub(crate) fn present_on_then(&self) -> bool { self.guards[0].present_on_then }
    pub(crate) fn guards(&self) -> &[GuardedScalarBranch] { &self.guards }
    pub(crate) fn alias_accesses(&self) -> &[usize] { &self.alias_accesses }
}

#[derive(Clone, Copy)]
enum ExpressionReadMode {
    Ordinary,
    ProjectedOrigins,
    GuardedScalarAggregateInitializer,
    OrdinaryScalarAggregateInitializer,
    FallibleAliasInitializer,
    DereferenceOperand,
    ArrayLengthArgument,
    GuardedWholeReleaseArgument,
    WidenedBorrowAddress(ByteSpan),
    WidenedOwningRoundtrip(ByteSpan),
    FixedCopyOwningArgument(ByteSpan, u64),
}

const MAX_JOINED_REBORROW_PARENTS: usize = 4;
const MAX_JOINED_REBORROW_DESCENDANTS: usize = 4;
const MAX_JOINED_REBORROW_ALIASES: usize = 8;

pub(super) struct ReferenceFlowBuilder<'a> {
    capture_call_points: bool,
    capture_private_scalar_tail: bool,
    private_scalar_tail_active: bool,
    root_return_points: std::ops::Range<usize>,
    mixed_owners: Option<&'a MixedReceiptAuthority>,
    shared_scalar_read_effects: Option<&'a HashMap<ByteSpan, SharedScalarReadEffect>>,
    local_borrow: Option<local_borrow::Author>,
    aggregate_facts: aggregate::AggregateFlowFacts,
    aggregate_result: aggregate::AggregateFlowResult,
    returning_leaf_prefix: bool,
    formal_forward_shape: Option<(Option<CfgPointId>, Option<CfgPointId>)>,
    independent_checked_query: bool,
    checked_scalar_storage: BTreeSet<ReferenceBindingId>,
    checked_result_origins: BTreeSet<ReferenceOriginId>,
    proof_points: CallablePointFacts,
    has_projected_child: bool,
    deferred_reborrow_roots: Vec<(ReferenceBindingId, super::control::DeferredReborrowRoot, CfgPointId, ReferenceOriginId)>,
    deferred_local_reborrow: bool,
    joined_local_reborrow: bool,
    has_joined_root_reborrow: bool,
    joined_mutable_child: Option<ReferenceBindingId>,
    joined_reborrow_call_admission: Option<bool>,
    pub(super) source: SourceIdentity,
    pub(super) function_span: ByteSpan,
    pub(super) names: BTreeMap<String, ReferenceBindingId>,
    pub(super) module_names: &'a HashSet<String>,
    pub(super) call_sites: &'a FunctionCallSites,
    pub(super) origins: HashMap<ReferenceBindingId, BTreeSet<ReferenceOriginId>>,
    pub(super) checked: HashSet<ReferenceBindingId>,
    pub(super) binding_types: ReferenceBindingTypes,
    pub(super) structs: &'a [ScalarStruct],
    pub(super) enums: &'a [ScalarEnum],
    pub(super) generic_parameters: BTreeSet<String>,
    pub(super) dereference_points: HashMap<OccurrenceOrdinal, CfgPointId>,
    origin_index: ReferenceOriginIndex,
    pub(super) cfg: ScalarReferenceCfg,
    widened_arrays: Option<HashSet<ReferenceBindingId>>,
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

struct ReferenceConditionalEdges {
    branch: CfgPointId,
    terminals: [CfgPointId; 2],
    join: CfgPointId,
}

impl ReferenceConditionalEdges {
    fn connect(&self, points: &mut [ReferenceFlowPoint]) {
        for terminal in self.terminals {
            points[terminal.0].successors.push(self.join);
        }
    }

    fn entries<'a>(&self, points: &'a [ReferenceFlowPoint]) -> Option<&'a [CfgPointId; 2]> {
        points[self.branch.0].successors.as_slice().try_into().ok()
    }
}

#[cfg(test)]
#[test]
fn reference_conditional_connector_preserves_then_and_unit_bypass_edges() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 1);
    let point = |id, kind, scope, successors| ReferenceFlowPoint {
        id: CfgPointId(id), kind, source_span: SourceSpan::new(source.clone(), span),
        scope: ReferenceScopeId(scope), possible_origins: None, successors,
    };
    let mut points = [
        point(0, CfgPointKind::Branch { condition: ScalarExpression::Boolean { value: true, span } }, 0, vec![CfgPointId(1)]),
        point(1, CfgPointKind::ScopeEnter, 1, vec![CfgPointId(2)]),
        point(2, CfgPointKind::ScopeExit, 1, Vec::new()),
        point(3, CfgPointKind::Join, 0, Vec::new()),
    ];
    let edges = ReferenceConditionalEdges {
        branch: CfgPointId(0), terminals: [CfgPointId(2), CfgPointId(0)], join: CfgPointId(3),
    };
    assert!(edges.entries(&points).is_none());
    edges.connect(&mut points);
    assert_eq!(edges.entries(&points), Some(&[CfgPointId(1), CfgPointId(3)]));
    assert_eq!(points[1].successors, [CfgPointId(2)]);
    assert_eq!(points[2].successors, [CfgPointId(3)]);
    assert!(points[3].successors.is_empty());
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
        let local_reborrow = deferred_named_reborrow_allowed(function, call_sites)
            && call_sites.boundary.parameters.iter().all(|slot| aggregate::ordinary_value(&slot.ty));
        Self {
            capture_call_points: false,
            // Store only exact CFG call ordinals during the traversal already
            // required for this body. The typed proof is built on demand.
            capture_private_scalar_tail: super::typecheck::is_module_private_name(&function.name)
                && function.generic_parameters.is_empty() && function.overload_arms.is_empty()
                && matches!(&call_sites.plain_names, PlainNameFacts::Prepared(_))
                && !call_sites.call_occurrences.is_empty(),
            private_scalar_tail_active: false,
            widened_arrays: None,
            root_return_points: 0..0,
            mixed_owners: None,
            shared_scalar_read_effects: None,
            local_borrow: (matches!(&call_sites.named_place_roots, NamedPlaceRootFacts::Prepared(roots)
                if !roots.is_empty() || (call_sites.conditionals.len() == 1
                    && matches!(call_sites.conditionals.values().next(), Some(ConditionalRegion::Unit { .. }))))
                && call_sites.boundary.root_outputs.is_empty()
                && call_sites.boundary.parameters.iter().all(|parameter| aggregate::ordinary_value(&parameter.ty))
                && function.generic_parameters.is_empty() && call_sites.call_occurrences.is_empty()
                && call_sites.structure.module_storage_uses.is_empty())
                .then(local_borrow::Author::default),
            aggregate_facts: aggregate::AggregateFlowFacts::default(),
            aggregate_result: aggregate::AggregateFlowResult::default(),
            returning_leaf_prefix: true,
            formal_forward_shape: None,
            independent_checked_query: aggregate::checked_scalar_output(&call_sites.boundary.root_outputs),
            checked_scalar_storage: BTreeSet::new(),
            checked_result_origins: BTreeSet::new(),
            proof_points: CallablePointFacts {
                has_grouped_assignment_capture: Some(false),
                scope_exit_bindings: matches!(&call_sites.named_place_roots, NamedPlaceRootFacts::Prepared(_))
                    .then(HashMap::new),
                ..CallablePointFacts::default()
            },
            has_projected_child: false,
            deferred_reborrow_roots: Vec::new(),
            deferred_local_reborrow: local_reborrow && call_sites.call_occurrences.is_empty(),
            joined_local_reborrow: local_reborrow,
            has_joined_root_reborrow: false,
            joined_mutable_child: None,
            joined_reborrow_call_admission: None,
            source: source.clone(),
            function_span: function.span,
            names: BTreeMap::new(),
            module_names,
            call_sites,
            origins: HashMap::new(),
            checked: HashSet::new(),
            binding_types: ReferenceBindingTypes::default(),
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
                observed_runtime_allocations: None,
                guarded_scalar_allocation: None,
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
            .or_else(|| self.call_sites.allocation_length_call_targets.get(&ordinal).cloned())
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
        if let Some(author) = &mut self.local_borrow {
            if !author.emit(id, &kind, scope, &self.binding_types) {
                self.local_borrow = None;
            }
        }
        self.formal_forward_shape = self.formal_forward_shape.and_then(|(moved, call)| {
            match &kind {
                CfgPointKind::Move { place, .. }
                    if moved.is_none() && call.is_none() && place.projections.is_empty() =>
                    Some((Some(id), None)),
                CfgPointKind::Call { output: 0, target: ResolvedCallTarget::ModuleCallable { .. },
                    argument_points, .. }
                    if moved.is_some() && call.is_none() && (1..=4).contains(&argument_points.len()) =>
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

    // The original authored expressions still execute and still lower as
    // written. A cloned proof place captures a sealed occurrence value for
    // overlap/availability only, without retaining a lookup into mutable state.
    fn proof_place<'p>(&self, place: &'p ScalarPlace, mut ordinal: OccurrenceOrdinal) -> std::borrow::Cow<'p, ScalarPlace> {
        use std::borrow::Cow;
        let structure = &self.call_sites.structure;
        if structure.proven_index_values.is_empty() {
            return Cow::Borrowed(place);
        }
        let mut current = place;
        let mut depth = 0;
        let mut replacements = Vec::new();
        loop {
            let (base, index) = match current {
                ScalarPlace::Field { base, .. } => (base, None),
                ScalarPlace::Index { base, index, .. } => (base, Some(index)),
                _ => break,
            };
            let parent = OccurrenceParent::Place(ordinal);
            ordinal = self.place_ordinal(parent, OccurrenceEdge::PlaceBase);
            if let Some(index) = index {
                let key = self.expression_ordinal(parent, OccurrenceEdge::PlaceIndex);
                if let (Some(&value), Some(ScalarType::Array { element, length, .. })) =
                    (structure.proven_index_values.get(&key), structure.checked_types.places.get(&ordinal))
                {
                    if aggregate::ordinary_value(element) && value < *length {
                        replacements.push((depth, value, expression_span(index)));
                    }
                }
            }
            current = base;
            depth += 1;
        }
        if replacements.is_empty() {
            return Cow::Borrowed(place);
        }
        let mut result = place.clone();
        let mut current = &mut result;
        let mut replacements = replacements.into_iter().peekable();
        let mut depth = 0;
        while let Some(&(replace_depth, value, span)) = replacements.peek() {
            current = match current {
                ScalarPlace::Field { base, .. } => base,
                ScalarPlace::Index { base, index, .. } => {
                    if depth == replace_depth {
                        **index = ScalarExpression::Integer { value: value.into(), span };
                        replacements.next();
                    }
                    base
                }
                _ => unreachable!("proved index was collected from this exact place"),
            };
            depth += 1;
        }
        Cow::Owned(result)
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

    fn prepared_fallible_alias(
        &self,
        binding: &crate::ScalarBinding,
        expression: &ScalarExpression,
        ordinal: OccurrenceOrdinal,
        scope: ReferenceScopeId,
        expected: &ScalarType,
    ) -> Option<ReferenceBindingId> {
        let sites = self.proof_points.fallible_allocation_bindings.as_ref()?;
        if binding.receivers.len() != 1 || binding.fallible_assignment.is_some()
            || binding.output_origin != ScalarBindingOutputOrigin::SingleExpression
        {
            return None;
        }
        let ScalarExpression::CheckedAddress { mutability, place, .. } = expression else { return None };
        let ScalarType::CheckedReference { mutability: expected_mode, inner } = expected else { return None };
        if mutability != expected_mode
            || self.call_sites.structure.checked_types.expressions.get(&ordinal) != Some(expected)
        {
            return None;
        }
        let occurrence = self.place_ordinal(OccurrenceParent::Expression(ordinal), OccurrenceEdge::ExpressionPlace);
        let root = self.place_root(place, occurrence);
        let NamedPlaceRootFacts::Prepared(roots) = &self.call_sites.named_place_roots else { return None };
        let NamedPlaceRootIdentity::Lexical(owner) = roots.get(&root.ordinal)? else { return None };
        let site = sites.get(owner)?;
        match place {
            ScalarPlace::Name { .. } if site.scope == scope && site.ty == **inner
                && (is_integer_type(inner) || site.whole_copy)
                && self.call_sites.structure.checked_types.places.get(&occurrence) == Some(inner.as_ref()) =>
                Some(owner.clone()),
            // A projected alias never serves as a root presence test. The
            // Borrow CFG point below must already see the same owner present.
            ScalarPlace::Index { .. } if aggregate::ordinary_value(inner)
                && self.exact_guarded_index(place, occurrence, owner, site) == Some(inner.as_ref()) =>
                Some(owner.clone()),
            _ => None,
        }
    }

    fn reject_fallible_address(
        &self,
        place: &ScalarPlace,
        ordinal: OccurrenceOrdinal,
        source: ByteSpan,
    ) -> Result<(), ReferenceAnalysisError> {
        let Some(bindings) = &self.proof_points.fallible_allocation_bindings else {
            return Ok(());
        };
        let root = self.place_root(place, ordinal);
        if let ScalarPlace::Dereference { pointer, .. } = root.place {
            let operand = self.expression_ordinal(OccurrenceParent::Place(root.ordinal), OccurrenceEdge::DereferencePointer);
            if let ScalarExpression::Name { span, .. } = pointer.as_ref() {
                if let Some(PlainNameIdentity::Lexical(alias)) = self.prepared_name_identity(operand, *span)? {
                    if self.proof_points.fallible_aliases.contains_key(alias) {
                        return Err(self.missing_origin(source, alias.clone(), ReferenceAnalysisErrorReason::UnsupportedFallibleAllocationAccess));
                    }
                }
            }
        }
        let ScalarPlace::Name { span, .. } = root.place else { return Ok(()) };
        if let Some(PreparedPlaceRoot::Lexical(binding)) = self.prepared_place_root(root.ordinal, *span)? {
            if bindings.contains_key(binding) || self.proof_points.fallible_aliases.contains_key(binding)
                || self.proof_points.guarded_runtime_receivers.contains(binding) {
                return Err(self.missing_origin(
                    source, binding.clone(), ReferenceAnalysisErrorReason::UnsupportedFallibleAllocationAccess,
                ));
            }
        }
        Ok(())
    }

    fn guarded_fallible_binding(
        &self,
        condition: &ScalarExpression,
        ordinal: OccurrenceOrdinal,
    ) -> Result<Option<(ReferenceBindingId, bool)>, ReferenceAnalysisError> {
        let Some(fallible_bindings) = &self.proof_points.fallible_allocation_bindings else {
            return Ok(None);
        };
        if self.call_sites.structure.checked_types.expressions.get(&ordinal) != Some(&ScalarType::Bool) {
            return Ok(None);
        }
        let ScalarExpression::Binary { operator, left, right, .. } = condition else {
            return Ok(None);
        };
        let present_on_then = match operator {
            BinaryOperator::Equal => false,
            BinaryOperator::NotEqual => true,
            _ => return Ok(None),
        };
        let parent = OccurrenceParent::Expression(ordinal);
        for (address, address_edge, null, null_edge) in [
            (left.as_ref(), OccurrenceEdge::BinaryLeft, right.as_ref(), OccurrenceEdge::BinaryRight),
            (right.as_ref(), OccurrenceEdge::BinaryRight, left.as_ref(), OccurrenceEdge::BinaryLeft),
        ] {
            let ScalarExpression::Name { span: null_span, .. } = null else { continue };
            let null = self.expression_ordinal(parent, null_edge);
            if !matches!(self.prepared_name_identity(null, *null_span)?, Some(PlainNameIdentity::Null)) {
                continue;
            }
            if let ScalarExpression::Name { span, .. } = address {
                let alias = self.expression_ordinal(parent, address_edge);
                if let Some(PlainNameIdentity::Lexical(alias)) = self.prepared_name_identity(alias, *span)? {
                    if let Some(owner) = self.proof_points.fallible_aliases.get(alias) {
                        if fallible_bindings.get(owner).is_some_and(|site| !site.has_indexed_extent()) {
                            return Ok(Some((owner.clone(), present_on_then)));
                        }
                    }
                }
                continue;
            }
            let ScalarExpression::CheckedAddress { mutability: ScalarReferenceMutability::Shared, place, .. } = address else {
                continue;
            };
            let ScalarPlace::Name { span: place_span, .. } = place else { continue };
            let address = self.expression_ordinal(parent, address_edge);
            let place = self.place_ordinal(OccurrenceParent::Expression(address), OccurrenceEdge::ExpressionPlace);
            if let Some(PreparedPlaceRoot::Lexical(binding)) = self.prepared_place_root(place, *place_span)? {
                if fallible_bindings.contains_key(binding) {
                    return Ok(Some((binding.clone(), present_on_then)));
                }
            }
        }
        Ok(None)
    }

    fn conditional_branches(
        &mut self,
        condition: &ScalarExpression,
        then_branch: &ScalarBlock,
        else_branch: Option<&ScalarBlock>,
        scope: ReferenceScopeId,
        region: &ConditionalRegion,
        projected_origins: bool,
        observe_fallible: bool,
    ) -> Result<([ReferenceConditionalArm; 2], Option<CfgPointId>, CfgPointId, Option<(ReferenceBindingId, bool)>), ReferenceAnalysisError> {
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
        let guarded = if observe_fallible {
            self.guarded_fallible_binding(condition, condition_ordinal)?
        } else {
            None
        };
        if guarded.is_some() {
            self.local_borrow = None;
            self.point(CfgPointKind::Evaluate(condition.clone()), expression_span(condition), scope, BTreeSet::new());
        } else {
            self.expression(condition, scope, condition_ordinal)?;
        }
        let condition_point = self.cursor;
        let branch = self.point(
            CfgPointKind::Branch {
                condition: condition.clone(),
            },
            expression_span(condition),
            scope,
            BTreeSet::new(),
        );
        let incoming_names = self.names.clone();
        let incoming_origins = else_branch.map(|_| self.origins.clone());
        let incoming_checked = self.checked.clone();
        let then_arm = ReferenceConditionalArm {
            result: self.block_result(then_branch, then_block, projected_origins)?,
            end: self.cursor.expect("then branch terminal"),
        };
        let then_checked = std::mem::replace(&mut self.checked, incoming_checked);
        self.names = incoming_names;
        self.cursor = Some(branch);
        let else_arm = match (else_branch, else_block) {
            (Some(else_branch), Some(else_block)) => {
                self.origins = incoming_origins.expect("explicit else entry origins");
                self.author_local_second_arm(CfgPointId(self.cfg.points.len()));
                ReferenceConditionalArm {
                    result: self.block_result(else_branch, else_block, projected_origins)?,
                    end: self.cursor.expect("else branch terminal"),
                }
            }
            (None, None) => ReferenceConditionalArm {
                result: ReferenceBlockResult::default(),
                end: branch,
            },
            _ => unreachable!("conditional syntax and region disagree on else arm"),
        };
        self.checked.extend(then_checked);
        self.origins.clear();
        self.cursor = None;
        Ok(([then_arm, else_arm], condition_point, branch, guarded))
    }

    fn author_local_second_arm(&mut self, entry: CfgPointId) {
        if self.local_borrow.as_mut().is_some_and(|author| !author.second_arm(entry)) {
            self.local_borrow = None;
        }
    }

    fn connect_reference_conditional(&mut self, edges: &ReferenceConditionalEdges, region: &ConditionalRegion) {
        edges.connect(&mut self.cfg.points);
        if self.local_borrow.as_mut().is_some_and(|author| {
            !author.fork(edges, &self.cfg.points, matches!(region, ConditionalRegion::Value { .. }))
        }) {
            self.local_borrow = None;
        }
    }

    fn record_conditional_inputs(&mut self, edges: &ReferenceConditionalEdges, expression: OccurrenceOrdinal) {
        if let Some(&entries) = edges.entries(&self.cfg.points) {
            let receipt = aggregate::ConditionalInputRegion {
                expression, branch: edges.branch, entries, terminals: edges.terminals, join: edges.join,
            };
            self.aggregate_result.conditional_input_regions.insert(edges.branch, receipt);
        }
    }

    fn seal_local_borrow(&mut self) {
        self.proof_points.local_borrow = self.local_borrow.take()
            .filter(|_| self.proof_points.pending_named_reborrow.is_none())
            .and_then(|author| author.seal(&self.cfg.points));
    }

    fn author_local_scope_exit(&mut self, point: CfgPointId, leaving: &super::control::ScopeExitBindings, prepared: bool) {
        if let Some(author) = &mut self.local_borrow {
            if !prepared || !author.end_scope(point, leaving) {
                self.local_borrow = None;
            }
        }
    }

    fn conditional(
        &mut self,
        condition: &ScalarExpression,
        then_branch: &ScalarBlock,
        else_branch: Option<&ScalarBlock>,
        span: ByteSpan,
        scope: ReferenceScopeId,
        region: &ConditionalRegion,
        expression: OccurrenceOrdinal,
        projected_origins: bool,
        checked_result: bool,
        array_return: bool,
        literal_unit: Option<(OccurrenceOrdinal, aggregate::ArrayWriterSide, Option<ReferenceBindingId>)>,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let condition_predecessor = self.cursor;
        let ([then_arm, else_arm], condition_point, branch, guarded) = self.conditional_branches(
            condition,
            then_branch,
            else_branch,
            scope,
            region,
            projected_origins,
            true,
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
        let edges = ReferenceConditionalEdges { branch, terminals: [then_arm.end, else_arm.end], join };
        self.connect_reference_conditional(&edges, region);
        self.record_conditional_inputs(&edges, expression);
        if let Some((binding, present_on_then)) = guarded {
            if let Some([then, otherwise]) = edges.entries(&self.cfg.points) {
                let condition = match region {
                    ConditionalRegion::Value { condition, .. } | ConditionalRegion::Unit { condition, .. } => *condition,
                };
                self.proof_points.record_guarded_allocation(branch, condition, binding, [*then, *otherwise], present_on_then);
            }
        }
        if let (Some((expression, writer_side, formal)), Some(condition), Some([then_entry, else_entry])) =
            (literal_unit, condition_point, edges.entries(&self.cfg.points))
        {
            let (writer_entry, retained_entry) = writer_side.split(*then_entry, *else_entry);
            let (writer_exit, retained_exit) = writer_side.split(edges.terminals[0], edges.terminals[1]);
            let edges = match region {
                ConditionalRegion::Unit { then_block, .. }
                    if writer_side == aggregate::ArrayWriterSide::Then && retained_entry == join =>
                    Some((*then_block, None)),
                ConditionalRegion::Value { then_block, else_block, .. }
                    if retained_entry != join && retained_entry != writer_entry =>
                {
                    let (writer_block, retained_block) = writer_side.split(*then_block, *else_block);
                    Some((writer_block, Some(aggregate::LiteralArrayRetained {
                        entry: retained_entry, exit: retained_exit, block: retained_block,
                        scope: self.cfg.points[retained_entry.0].scope,
                    })))
                }
                _ => None,
            };
            // Follow at most two operand edges in the CFG built by the normal
            // expression visitor, not a second expression or effects walk.
            let operands = match formal {
                None => Some(None),
                Some(formal) => condition_predecessor.and_then(|preceding| {
                    let [read] = self.cfg.points[preceding.0].successors.as_slice() else { return None };
                    let [literal] = self.cfg.points[read.0].successors.as_slice() else { return None };
                    (matches!(&self.cfg.points[read.0].kind, CfgPointKind::Read { binding } if binding == &formal)
                        && matches!(&self.cfg.points[literal.0].kind,
                            CfgPointKind::Evaluate(ScalarExpression::Integer { .. }))
                        && self.cfg.points[literal.0].successors.as_slice() == [condition])
                        .then_some(Some([*read, *literal]))
                }),
            };
            if let (Some((writer_block, retained)), Some(operands)) = (edges, operands) {
                let other_write = self.aggregate_facts.literal_array_other_write;
                if self.aggregate_facts.literal_array_other_writer.is_some() == other_write.is_some() {
                    self.aggregate_facts.literal_array_unit = Some(aggregate::LiteralArrayUnit {
                        expression, condition, operands, branch, writer_side, writer_entry, writer_exit,
                        writer_block, retained, other_write,
                        writer_scope: self.cfg.points[writer_entry.0].scope, join,
                    });
                }
            }
        }
        if array_return {
            self.record_conditional_array_return(join, 1, &then_arm, &else_arm);
        }
        Ok(if joined_value || checked_result {
            ReferenceValue::Uncomputed
        } else {
            ReferenceValue::Other
        })
    }

    fn record_conditional_array_return(
        &mut self,
        join: CfgPointId,
        arity: usize,
        then_arm: &ReferenceConditionalArm,
        else_arm: &ReferenceConditionalArm,
    ) {
        let arm_witness = |arm: &ReferenceConditionalArm| {
            let formal = self.aggregate_facts.array_formal.as_ref()?;
            if arm.result.output_points.len() != arity {
                return None;
            }
            let value = &self.cfg.points[arm.result.output_points.first()?.0];
            let end = &self.cfg.points[arm.end.0];
            if value.scope != end.scope {
                return None;
            }
            if arity == 1 && value.successors.as_slice() == [arm.end]
                && self.aggregate_facts.neutral_array_calls.contains(&value.id)
                && matches!(&value.kind, CfgPointKind::Call { output: 0,
                    target: ResolvedCallTarget::ModuleCallable { .. }, .. })
            {
                let (_, moved, binding) = self.aggregate_facts.array_calls.get(&value.id)?;
                return (binding == formal).then_some(aggregate::ConditionalArrayArm {
                    exit: arm.end,
                    moved: *moved,
                    tail_call: Some(value.id),
                });
            }
            let [moved] = value.successors.as_slice() else { return None };
            let moved = &self.cfg.points[moved.0];
            (moved.scope == end.scope
                && matches!(&value.kind, CfgPointKind::Read { binding } if binding == formal)
                && matches!(&moved.kind, CfgPointKind::Move { place, owner, ty: ScalarType::RuntimeArray { .. } }
                    if owner == formal && &place.binding == formal && place.projections.is_empty())
                && (arity != 1 || moved.successors.as_slice() == [arm.end]))
                .then_some(aggregate::ConditionalArrayArm { exit: arm.end, moved: moved.id, tail_call: None })
        };
        if let (Some(then_arm), Some(else_arm)) = (arm_witness(then_arm), arm_witness(else_arm)) {
            self.aggregate_facts.conditional_array_return = Some(aggregate::ConditionalArrayReturn {
                join,
                arms: [then_arm, else_arm],
            });
        }
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
        let ([then_arm, else_arm], _, branch, _) = self.conditional_branches(
            condition,
            then_branch,
            Some(else_branch),
            scope,
            region,
            true,
            false,
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
        let edges = ReferenceConditionalEdges { branch, terminals: [then_arm.end, else_arm.end], join: first };
        edges.connect(&mut self.cfg.points);
        self.record_conditional_inputs(&edges, primary.expression);
        if scope == ReferenceScopeId(0) && primary.output == 0
            && self.call_sites.boundary.root_outputs.len() == arity
        {
            if let ([first, second], ConditionalRegion::Value { then_block, else_block, .. }) =
                (row.types.as_slice(), region)
            {
                if let Some(lanes) = aggregate::mixed_owner_result_lanes(first, second) {
                    self.aggregate_facts.record_mixed_arm_projection(
                        primary.expression, result.output_points[lanes.owner], lanes.owner,
                        [*then_block, *else_block],
                        [then_arm.result.output_points[lanes.owner], else_arm.result.output_points[lanes.owner]],
                    );
                }
            }
        }
        // The projected row already binds every root slot to this producer and
        // its exact arm output. Only lane zero may carry the formal's backing.
        if scope == ReferenceScopeId(0) && primary.output == 0
            && self.aggregate_facts.array_formal.is_some()
            && matches!(row.types.first(), Some(ScalarType::RuntimeArray { .. }))
            && row.types.iter().skip(1).all(aggregate::array_neutral_scalar)
            && self.call_sites.boundary.root_outputs.len() == arity
        {
            self.record_conditional_array_return(first, arity, &then_arm, &else_arm);
        }
        Ok(result)
    }

    pub(super) fn expression(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        self.expression_with_read_mode(expression, scope, ordinal, ExpressionReadMode::Ordinary)
    }

    fn expression_with_read_mode(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
        read_mode: ExpressionReadMode,
    ) -> Result<ReferenceValue, ReferenceAnalysisError> {
        let before = self.cursor;
        let value = self.evaluate_expression(expression, scope, ordinal, read_mode)?;
        if self.capture_call_points && self.cursor != before {
            if let Some(point) = self.cursor {
                self.proof_points.evaluations.insert(ordinal, point);
            }
        }
        if let Some(point) = self.cursor.filter(|point| Some(*point) != before) {
            let existing_field = matches!(&self.cfg.points[point.0].kind,
                CfgPointKind::ReadPlace { place: ScalarPlace::Field { base, .. }, pointer_point: Some(_), .. }
                    if matches!(base.as_ref(), ScalarPlace::Dereference { .. }));
            if (existing_field || self.exact_scalar_receipt_dereference(point, ordinal))
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

    fn checked_array_unit_candidate(
        &mut self,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
        condition: &ScalarExpression,
        then_branch: &ScalarBlock,
        else_branch: Option<&ScalarBlock>,
        region: &ConditionalRegion,
    ) -> Option<(OccurrenceOrdinal, aggregate::ArrayWriterSide, Option<ReferenceBindingId>)> {
        let calls = self.call_sites;
        if scope != ReferenceScopeId(0) || self.aggregate_facts.literal_array_bind.is_none()
            || !calls.boundary.root_outputs.is_empty()
            || (else_branch.is_some()
                && !matches!(calls.structure.checked_types.expressions.get(&ordinal), Some(ScalarType::Unit)))
            || !matches!(calls.structure.blocks[calls.structure.root.0].items.get(1)
                .map(|item| &item.kind), Some(BlockItemKind::Evaluation { expression }) if *expression == ordinal)
        {
            return None;
        }
        use aggregate::ArrayWriterSide::{Else, Then};
        let (condition_ordinal, writer, side) = match (region, then_branch.items.len(), else_branch.map(|block| block.items.len())) {
            (ConditionalRegion::Unit { condition, then_block, .. }, 1, None)
            | (ConditionalRegion::Value { condition, then_block, .. }, 1, Some(0)) =>
                (*condition, *then_block, Then),
            (ConditionalRegion::Value { condition, else_block, .. }, 0, Some(1)) =>
                (*condition, *else_block, Else),
            (ConditionalRegion::Value { condition, then_block, .. }, 1, Some(1)) =>
                (*condition, *then_block, Then),
            _ => return None,
        };
        let types = &calls.structure.checked_types.expressions;
        if types.get(&condition_ordinal) != Some(&ScalarType::Bool) {
            return None;
        }
        let comparison_formal = match condition {
            ScalarExpression::Boolean { .. } => None,
            ScalarExpression::Name { .. } => {
                self.literal_array_formal_binding(condition_ordinal, &ScalarType::Bool)?;
                None
            }
            ScalarExpression::Binary { operator: BinaryOperator::Equal, left, right, .. }
                if matches!(left.as_ref(), ScalarExpression::Name { .. }) =>
            {
                let ScalarExpression::Integer { value, .. } = right.as_ref() else { return None };
                u64::try_from(value.clone()).ok()?;
                let parent = OccurrenceParent::Expression(condition_ordinal);
                let left = self.expression_ordinal(parent, OccurrenceEdge::BinaryLeft);
                let right = self.expression_ordinal(parent, OccurrenceEdge::BinaryRight);
                if types.get(&right) != Some(&ScalarType::U64) {
                    return None;
                }
                Some(self.literal_array_formal_binding(left, &ScalarType::U64)?.clone())
            }
            _ => return None,
        };
        self.aggregate_facts.literal_array_writer = Some(writer);
        if let (ConditionalRegion::Value { else_block, .. }, 1, Some(1)) =
            (region, then_branch.items.len(), else_branch.map(|block| block.items.len()))
        {
            self.aggregate_facts.literal_array_other_writer = Some(*else_block);
        }
        Some((ordinal, side, comparison_formal))
    }

    fn evaluate_expression(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
        read_mode: ExpressionReadMode,
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
                let literal_unit = self.checked_array_unit_candidate(
                    scope, ordinal, condition, then_branch, Some(else_branch), region,
                );
                self.conditional(
                    condition,
                    then_branch,
                    Some(else_branch),
                    span,
                    scope,
                    region,
                    ordinal,
                    matches!(read_mode, ExpressionReadMode::ProjectedOrigins),
                    matches!(calls.structure.checked_types.expressions.get(&ordinal),
                        Some(ScalarType::CheckedReference { .. })),
                    scope == ReferenceScopeId(0) && self.aggregate_facts.array_formal.is_some()
                        && matches!(calls.structure.checked_types.expressions.get(&ordinal),
                            Some(ScalarType::RuntimeArray { .. }))
                        && matches!(calls.boundary.root_outputs.as_slice(),
                            [output] if matches!(output.ty, ScalarType::RuntimeArray { .. }))
                        && calls.structure.blocks[calls.structure.root.0].final_outputs()
                            .next().is_some_and(|slot| slot.output == 0 && slot.expression == ordinal),
                    literal_unit,
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
                let literal_unit = self.checked_array_unit_candidate(
                    scope, ordinal, condition, then_branch, None, region,
                );
                self.conditional(condition, then_branch, None, span, scope, region,
                    ordinal, false, false, false, literal_unit)
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
                if !self.independent_checked_query && calls.local_shared_call_families.get(&ordinal) == Some(&false) {
                    return Err(self.missing_context(span, ReferenceAnalysisErrorReason::UnprovedLocalSharedCallableFamily));
                }
                let mut address = None;
                let mut checked_release_address = None;
                let mut named_release_binding = None;
                let mut argument_points = Vec::new();
                let signature = calls.checked_call_signatures.get(&ordinal);
                let widened_borrow = (|| {
                    let owners = self.widened_arrays.as_ref()?;
                    if !self.generic_parameters.is_empty()
                        || calls.declared_unsafe_calls.contains(&ordinal.0)
                        || !matches!(target, ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. })
                    { return None; }
                    let [ScalarExpression::CheckedAddress { mutability: ScalarReferenceMutability::Shared,
                        place: ScalarPlace::Name { name, .. }, .. }] = arguments.as_slice() else { return None };
                    // The existing lexical inventory restores disjoint scopes.
                    // Normal prepared borrow replay decides whether this exact
                    // owner is active at the call; a sibling's owner is distinct.
                    let binding = self.names.get(name)?;
                    if !owners.contains(binding) {
                        return None;
                    }
                    let signature = signature?;
                    let [ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner }] =
                        signature.parameters.as_slice() else { return None };
                    let [output] = signature.outputs.outputs.as_slice() else { return None };
                    (output.ty == ScalarType::U64
                        && self.binding_types.get(binding).is_some_and(|ty| aggregate::same_runtime_array_type(ty, inner)))
                        .then_some(binding.declaration_span)
                })();
                let guarded_release = (|| {
                    if target != ResolvedCallTarget::Core(CoreOperationId::Free) { return None; }
                    let [ScalarExpression::CheckedAddress { mutability: ScalarReferenceMutability::Shared,
                        place: ScalarPlace::Name { span, .. }, .. }] = arguments.as_slice() else { return None; };
                    let argument = self.expression_ordinal(parent, OccurrenceEdge::CallArgument(0));
                    let place = self.place_ordinal(OccurrenceParent::Expression(argument), OccurrenceEdge::ExpressionPlace);
                    let PreparedPlaceRoot::Lexical(binding) = self.prepared_place_root(place, *span).ok().flatten()? else { return None; };
                    self.proof_points.fallible_allocation_bindings.as_ref()?.get(binding)?
                        .has_runtime_extent().then(|| binding.clone())
                })();
                let mut borrowed_extent_formal = None;
                let mut aggregate_arguments = BTreeMap::new();
                let mut array_argument = None;
                let mut raw_arguments = Vec::new();
                for (position, argument) in arguments.iter().enumerate() {
                    let ordinal =
                        self.expression_ordinal(parent, OccurrenceEdge::CallArgument(position));
                    let array_length = target == ResolvedCallTarget::Core(CoreOperationId::ArrayLen);
                    if array_length {
                        self.expression_with_read_mode(argument, scope, ordinal, ExpressionReadMode::ArrayLengthArgument)?;
                    } else if guarded_release.is_some() {
                        self.expression_with_read_mode(argument, scope, ordinal, ExpressionReadMode::GuardedWholeReleaseArgument)?;
                    } else if let Some(owner) = widened_borrow {
                        self.expression_with_read_mode(
                            argument, scope, ordinal, ExpressionReadMode::WidenedBorrowAddress(owner),
                        )?;
                    } else {
                        self.expression(argument, scope, ordinal)?;
                    }
                    let fixed_copy = if let (ScalarExpression::Name { .. }, Some(parameter @ ScalarType::RuntimeArray { .. })) =
                        (argument, signature.and_then(|signature| signature.parameters.get(position)))
                    {
                        calls.structure.checked_types.expressions.get(&ordinal)
                            .and_then(|actual| super::fixed_copy_to_runtime(parameter, actual))
                    } else { None };
                    if let Some((_, count)) = fixed_copy
                    {
                        let point = self.cursor.expect("fixed Copy argument");
                        if !matches!(read_mode, ExpressionReadMode::FixedCopyOwningArgument(owner, actual)
                            if position == 0 && actual == count
                                && matches!(&self.cfg.points[point.0].kind, CfgPointKind::Read { binding }
                                    if binding.declaration_span == owner))
                        {
                            return Err(self.missing_context(expression_span(argument),
                                ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer));
                        }
                    }
                    argument_points.push(self.cursor.expect("evaluated call argument"));
                    if array_length {
                        let Some(CfgPointKind::Borrow { place, .. }) =
                            self.cursor.map(|id| &self.cfg.points[id.0].kind)
                        else {
                            return Err(self.missing_context(span, ReferenceAnalysisErrorReason::UnprovedArrayLength));
                        };
                        match self.binding_types.get(&place.binding) {
                            _ if matches!(place.place, ScalarPlace::Dereference { .. })
                                && matches!(place.projections.as_slice(), [ReferencePlaceProjection::Dereference(_)])
                                && signature.is_some_and(|signature| matches!(signature.parameters.as_slice(),
                                    [ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner }]
                                        if matches!(inner.as_ref(), ScalarType::Array { .. }))) => {}
                            Some(ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner })
                                if matches!(inner.as_ref(), ScalarType::RuntimeArray { .. })
                                    && matches!(place.place, ScalarPlace::Dereference { .. })
                                    && matches!(place.projections.as_slice(), [ReferencePlaceProjection::Dereference(_)])
                                    && matches!(calls.boundary.parameters.as_slice(), [formal]
                                        if formal.span == place.binding.declaration_span)
                                    && matches!(calls.boundary.root_outputs.as_slice(), [output] if output.ty == ScalarType::U64) =>
                            {
                                borrowed_extent_formal = Some(place.binding.declaration_span);
                            }
                            Some(ScalarType::Array { .. }) if place.projections.is_empty() => {}
                            Some(ScalarType::RuntimeArray { .. }) if place.projections.is_empty()
                                && self.proof_points.fallible_allocation_bindings.as_ref()
                                    .and_then(|sites| sites.get(&place.binding)).is_some_and(|site|
                                        site.has_runtime_extent()) => {}
                            Some(ScalarType::RuntimeArray { .. }) if place.projections.is_empty()
                                && self.proof_points.ordinary_array_receivers.contains_key(&place.binding) => {
                                self.aggregate_result.ordinary_array_extent_demands.insert(
                                    self.proof_points.ordinary_array_receivers[&place.binding]);
                                self.proof_points.array_length_owners.insert(place.binding.clone());
                            }
                            Some(ScalarType::RuntimeArray { .. }) if place.projections.is_empty()
                                && self.cfg.allocations.contains(&place.binding) => {
                                self.proof_points.array_length_owners.insert(place.binding.clone());
                            }
                            Some(ScalarType::RuntimeArray { .. }) if place.projections.is_empty()
                                && self.aggregate_facts.array_formal.as_ref() == Some(&place.binding)
                                && matches!(calls.boundary.parameters.as_slice(), [_])
                                && matches!(calls.boundary.root_outputs.as_slice(), [output]
                                    if matches!(output.ty, ScalarType::RuntimeArray { .. })) => {
                                // The access is safe only as part of an all-use paired
                                // signature. Carry a provisional exact borrow witness.
                                self.aggregate_facts.array_formal_extent_borrows
                                    .insert(self.cursor.expect("checked array extent argument"));
                            }
                            _ => return Err(self.missing_context(span, ReferenceAnalysisErrorReason::UnprovedArrayLength)),
                        }
                    }
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
                    let exact_widened_move = matches!(read_mode, ExpressionReadMode::WidenedOwningRoundtrip(owner)
                        if matches!(argument, ScalarExpression::Name { span, .. }
                            if matches!(self.prepared_name_identity(ordinal, *span),
                                Ok(Some(PlainNameIdentity::Lexical(binding))) if binding.declaration_span == owner)));
                    self.move_value_with_widened_receipt(argument, scope, ordinal, exact_widened_move)?;
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
                        let release = self.point(
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
                        if let Some(owner) = guarded_release {
                            self.proof_points.guarded_releases.insert(release, owner);
                        }
                        if self.joined_local_reborrow {
                            self.proof_points.joined_checked_releases.push(release);
                        }
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
                if let Some(owner) = widened_borrow {
                    if !matches!(argument_points.as_slice(), [argument]
                        if self.borrowed_extent_argument(*argument, 0) == Some((owner, None)))
                    {
                        return Err(self.missing_context(span, ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer));
                    }
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
                let mixed_result = signature.and_then(|signature|
                    aggregate::mixed_owned_struct_result(&signature.outputs));
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
                let read_effect_argument = if !calls.declared_unsafe_calls.contains(&ordinal.0)
                {
                    (|| {
                        let CfgPointKind::Call { target, argument_points, .. } = &call else { return None };
                        let joined_effect = shared_scalar_read_effect(&self.source, calls, ordinal, target, self.shared_scalar_read_effects);
                        let effect = joined_effect.or_else(|| {
                            let ResolvedCallTarget::ModuleCallable {
                                source, declaration_span, concrete: ConcreteCallSelection::Declaration,
                            } = target else { return None };
                            if source == &self.source {
                                return self.shared_scalar_read_effects?.get(declaration_span).copied()
                                    .filter(|effect| effect.descriptor);
                            }
                            let (identity, effect) = calls.imported_shared_readers.get(&ordinal)?;
                            (&identity.source == source && identity.range == *declaration_span).then_some(*effect)
                        })?;
                        let parameters = &signature?.parameters;
                        let parameter = parameters.get(effect.parameter)?;
                        let ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner } = parameter else {
                            return None;
                        };
                        if parameters.len() != effect.parameter + 1
                            || (effect.parameter == 1 && (effect.descriptor || parameters.first() !=
                                Some(if effect.bool_prefix { &ScalarType::Bool } else { inner.as_ref() })))
                            || effect.parameter > 1
                        {
                            return None;
                        }
                        let [output] = signature?.outputs.outputs.as_slice() else { return None };
                        if if effect.returns_formal {
                            effect.parameter != 0 || &output.ty != parameter || !aggregate::ordinary_value(inner)
                        } else if effect.descriptor {
                            !matches!(inner.as_ref(), ScalarType::Struct(_)) || !aggregate::ordinary_value(&output.ty)
                        } else { &output.ty != inner.as_ref() || !aggregate::ordinary_value(inner) } {
                            return None;
                        }
                        let argument = self.expression_ordinal(parent, OccurrenceEdge::CallArgument(effect.parameter));
                        let point = *argument_points.get(effect.parameter)?;
                        if calls.structure.checked_types.expressions.get(&argument) != Some(parameter) {
                            return None;
                        }
                        let actual = self.cfg.points.get(point.0)?;
                        match &actual.kind {
                            CfgPointKind::Read { binding }
                                if matches!(calls.prepared_plain_name(argument),
                                    Some(PlainNameIdentity::Lexical(expected)) if expected == binding) =>
                                Some((point, joined_effect.is_some(), effect.parameter == 0 && !effect.joined_only && !effect.returns_formal,
                                    effect.returns_formal)),
                            CfgPointKind::Borrow { place, loan }
                                if effect.parameter == 0 && !effect.joined_only && !effect.returns_formal
                                    && matches!(target, ResolvedCallTarget::ModuleCallable { .. })
                                    && loan.mode == ScalarReferenceMutability::Shared && loan.parent.is_none()
                                    && loan.origin_place == *place =>
                            {
                                let ScalarExpression::CheckedAddress {
                                    mutability: ScalarReferenceMutability::Shared,
                                    place: written, span: written_span,
                                } = arguments.get(effect.parameter)? else { return None };
                                let (ScalarPlace::Name { span, .. }, projections) = (match written {
                                    named @ ScalarPlace::Name { .. } if effect.descriptor => (named, 0),
                                    ScalarPlace::Field { base, field: ScalarFieldReference::Resolved(_), .. }
                                        if !effect.descriptor => (base.as_ref(), 1),
                                    _ => return None,
                                }) else { return None };
                                let place_ordinal = self.place_ordinal(
                                    OccurrenceParent::Expression(argument), OccurrenceEdge::ExpressionPlace,
                                );
                                let root = self.place_root(written, place_ordinal);
                                (actual.source_span.range == *written_span && place.place == *written
                                    && place.projections.len() == projections
                                    && matches!(self.prepared_place_root(root.ordinal, *span).ok().flatten(),
                                        Some(PreparedPlaceRoot::Lexical(binding)) if binding == &place.binding))
                                    .then_some((point, false, true, false))
                            }
                            _ => None,
                        }
                    })()
                } else { None };
                let exclusive_write = if self.joined_local_reborrow && calls.exclusive_scalar_writers.contains(&ordinal) {
                    (|| {
                        let CfgPointKind::Call { argument_points, .. } = &call else { return None };
                        let [value, argument] = argument_points.as_slice() else { return None };
                        let read = self.cfg.points.get(argument.0)?;
                        let scalar = self.cfg.points.get(value.0)?;
                        let CfgPointKind::Read { binding } = &read.kind else { return None };
                        let moved = self.cursor?;
                        let moved_point = self.cfg.points.get(moved.0)?;
                        let parameter = signature?.parameters.get(1)?;
                        let argument_ordinal = self.expression_ordinal(parent, OccurrenceEdge::CallArgument(1));
                        let scalar_ordinal = self.expression_ordinal(parent, OccurrenceEdge::CallArgument(0));
                        let valid_scalar = match &scalar.kind {
                            CfgPointKind::Read { binding } => matches!(calls.prepared_plain_name(scalar_ordinal),
                                Some(PlainNameIdentity::Lexical(expected)) if expected == binding),
                            CfgPointKind::Evaluate(ScalarExpression::Integer { .. } | ScalarExpression::Float { .. }
                                | ScalarExpression::Boolean { .. } | ScalarExpression::Char { .. }) => true,
                            _ => false,
                        };
                        (valid_scalar && read.scope == scope && scalar.scope == scope && moved_point.scope == scope
                            && scalar.successors.as_slice() == [read.id]
                            && read.successors.as_slice() == [moved]
                            && calls.structure.checked_types.expressions.get(&scalar_ordinal) == signature?.parameters.first()
                            && calls.structure.checked_types.expressions.get(&argument_ordinal) == Some(parameter)
                            && matches!(calls.prepared_plain_name(argument_ordinal),
                                Some(PlainNameIdentity::Lexical(expected)) if expected == binding)
                            && matches!(&moved_point.kind, CfgPointKind::Move { owner, place, ty } if owner == binding
                                && &place.binding == binding && place.projections.is_empty() && ty == parameter)
                        ).then_some((moved, *argument))
                    })()
                } else { None };
                let id = self.point_with_origins(call, span, scope, cast_origins.clone());
                if let Some(owner) = widened_borrow {
                    self.aggregate_result.widened_borrowed_extent_calls.push((id, ordinal, owner));
                }
                if let Some(formal) = borrowed_extent_formal {
                    self.aggregate_facts.borrowed_extent_observer = Some((id, ordinal, formal));
                }
                if let Some((moved, argument)) = exclusive_write {
                    self.proof_points.exclusive_scalar_write_moves.push((moved, argument, id));
                }
                if let Some((argument, joined, receipt, returns_formal)) = read_effect_argument {
                    if receipt {
                        self.aggregate_facts.receipt_loan_read_calls.insert(id, (argument, ordinal));
                    }
                    if joined && self.joined_local_reborrow {
                        if returns_formal {
                            self.proof_points.joined_shared_return_calls.push((id, argument));
                        } else {
                            self.proof_points.shared_scalar_read_calls.push((id, argument));
                        }
                    }
                }
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
                } else if let Some(lanes) = mixed_result {
                    let selected = matches!(&self.cfg.points[id.0].kind, CfgPointKind::Call {
                        target: ResolvedCallTarget::ModuleCallable {
                            source, declaration_span, concrete: ConcreteCallSelection::Declaration,
                        }, ..
                    } if self.mixed_owners.is_some_and(|owners| owners.contains(source, declaration_span)));
                    self.aggregate_result.mixed_result_calls.insert(id);
                    if selected {
                        self.aggregate_facts.calls.insert(id, ordinal);
                        if lanes.owner == 1 {
                            self.aggregate_facts.secondary_owner_calls.insert(id);
                        }
                    }
                }
                if let Some(signature) = signature {
                    if signature.outputs.outputs.len() > 1 && signature.outputs.outputs.iter().all(|output|
                        matches!(output.ty, ScalarType::Struct(_)))
                    {
                        self.aggregate_result.multi_result_calls.insert(id);
                        if !calls.declared_unsafe_calls.contains(&ordinal.0) && matches!(&self.cfg.points[id.0].kind, CfgPointKind::Call {
                            target: ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete: ConcreteCallSelection::Declaration }, ..
                        } if self.mixed_owners.is_some_and(|owners| owners.is_multi(source, declaration_span))) {
                            self.aggregate_facts.multi_owner_calls.insert(id, (ordinal, signature.outputs.outputs.len()));
                            self.aggregate_result.multi_selected_calls.insert(id, (ordinal, signature.outputs.outputs.len()));
                        }
                    }
                }
                if !aggregate_arguments.is_empty() {
                    for (&slot, &provenance) in &aggregate_arguments {
                        self.aggregate_result.inputs.entry((id, ordinal.0, slot, provenance)).or_default();
                    }
                    if self.aggregate_facts.formals.len() == 1
                        && self.aggregate_facts.raw_formals.is_empty()
                        && !calls.declared_unsafe_calls.contains(&ordinal.0)
                        && matches!(&self.cfg.points[id.0].kind, CfgPointKind::Call {
                            target: ResolvedCallTarget::ModuleCallable {
                                concrete: ConcreteCallSelection::Declaration, ..
                            }, ..
                        })
                    {
                        for &value in aggregate_arguments.values() {
                            if let aggregate::ArgumentProvenance::WholeMove(moved) = value {
                                self.aggregate_facts.conditional_formal_moves.insert(moved, id);
                            }
                        }
                    }
                    self.aggregate_facts.input_moves.insert(id, (ordinal, aggregate_arguments));
                }
                if !raw_arguments.is_empty() {
                    self.aggregate_facts.raw_calls.insert(id, (raw_arguments, calls.declared_unsafe_calls.contains(&ordinal.0)));
                }
                if let Some(read_call) = read_call {
                    self.aggregate_facts.read_calls.insert(id, read_call);
                }
                if self.capture_call_points || self.private_scalar_tail_active {
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
                    // Operand evaluation is linear in this reference CFG, but the
                    // right operand is conditional in LLVM. Do not use its path
                    // as an exact ownership merge or dominating producer corridor.
                    if let Some(left_end) = self.cursor {
                        self.aggregate_result.conditional_input_barriers.insert(left_end);
                    }
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
            ScalarExpression::ValueField { receiver, .. } => {
                self.expression_child(receiver, scope, parent, OccurrenceEdge::ValueFieldReceiver)?;
                self.point(CfgPointKind::Evaluate(expression.clone()), span, scope, BTreeSet::new());
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
                let expected_element = match calls.structure.checked_types.expressions.get(&ordinal) {
                    Some(ScalarType::Array { element, length, .. })
                        if *length == elements.len() as u64 && aggregate::ordinary_value(element) => Some(element.as_ref()),
                    _ => None,
                };
                let guarded = matches!(read_mode, ExpressionReadMode::GuardedScalarAggregateInitializer);
                if guarded && !expected_element.is_some_and(|element| matches!(element, ScalarType::U64 | ScalarType::I32)) {
                    return Err(self.missing_context(span, ReferenceAnalysisErrorReason::UnprovedFallibleCallInitializer));
                }
                let mut ordinary = (matches!(read_mode, ExpressionReadMode::OrdinaryScalarAggregateInitializer)
                    && expected_element.is_some()).then(Vec::new);
                for (position, element) in elements.iter().enumerate() {
                    let ordinal =
                        self.expression_ordinal(parent, OccurrenceEdge::AggregateElement(position));
                    let before = self.cursor;
                    if guarded || ordinary.is_some() {
                        let exact_type = expected_element.filter(|ty|
                            calls.structure.checked_types.expressions.get(&ordinal) == Some(*ty));
                        if guarded && exact_type.is_none() {
                            return Err(self.missing_context(expression_span(element), ReferenceAnalysisErrorReason::UnprovedFallibleCallInitializer));
                        }
                        self.scalar_aggregate_member(element, scope, ordinal, exact_type, guarded, &mut ordinary)?;
                    } else {
                        self.expression(element, scope, ordinal)?;
                    }
                    if let Some(point) = self.cursor.filter(|point| Some(*point) != before) {
                        match calls.structure.checked_types.expressions.get(&ordinal) {
                            Some(ScalarType::RawPointer(_) | ScalarType::Struct(_)) if track_raw => operands.push(point),
                            Some(ScalarType::CheckedReference { .. }) => checked_inputs.push(point),
                            _ => {}
                        }
                    }
                    self.move_value(element, scope, ordinal)?;
                }
                if let Some(proofs) = ordinary.filter(|proofs| !proofs.is_empty()) {
                    self.aggregate_result.ordinary_fallible_expressions.push(proofs);
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
                let guarded = matches!(read_mode, ExpressionReadMode::GuardedScalarAggregateInitializer);
                let mut ordinary = matches!(read_mode, ExpressionReadMode::OrdinaryScalarAggregateInitializer)
                    .then(Vec::new);
                for (position, field) in fields.iter().enumerate() {
                    let ordinal =
                        self.expression_ordinal(parent, OccurrenceEdge::AggregateField(position));
                    let before = self.cursor;
                    let member = calls.structure.literal_fields.get(&ordinal);
                    let member_type = calls.structure.checked_types.expressions.get(&ordinal);
                    let exact_type = member.filter(|member| Some(&member.structure) == structure_id).and(member_type);
                    self.scalar_aggregate_member(&field.value, scope, ordinal, exact_type, guarded, &mut ordinary)?;
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
                if let Some(proofs) = ordinary.filter(|proofs| !proofs.is_empty()) {
                    self.aggregate_result.ordinary_fallible_expressions.push(proofs);
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
                self.reject_fallible_address(place, place_ordinal, span)?;
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
                if let Some(call) = exposed.as_ref().and_then(|owner| self.proof_points.ordinary_array_receivers.get(owner)) {
                    self.aggregate_result.ordinary_array_incompatible_uses.insert(*call);
                }
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
                // Whole reads, including values passed, captured, moved,
                // discarded or returned, are outside direct-backing enrollment.
                if let Some(call) = self.proof_points.ordinary_array_receivers.get(&binding) {
                    self.aggregate_result.ordinary_array_incompatible_uses.insert(*call);
                }
                if self.proof_points.guarded_runtime_receivers.contains(&binding) {
                    return Err(self.missing_origin(span, binding, ReferenceAnalysisErrorReason::UnsupportedFallibleAllocationAccess));
                }
                if let Some(owner) = self.proof_points.fallible_aliases.get(&binding) {
                    if !matches!(read_mode, ExpressionReadMode::DereferenceOperand) {
                        return Err(self.missing_origin(span, binding, ReferenceAnalysisErrorReason::UnsupportedFallibleAllocationAccess));
                    }
                    self.proof_points.fallible_alias_accesses.push((ordinal, owner.clone()));
                }
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
                let extent_only = matches!(read_mode, ExpressionReadMode::ArrayLengthArgument)
                    && *mutability == ScalarReferenceMutability::Shared && matches!(place, ScalarPlace::Name { .. });
                if !extent_only {
                    if let ScalarPlace::Name { span, .. } = root.place {
                        if let Some(PreparedPlaceRoot::Lexical(binding)) = self.prepared_place_root(root.ordinal, *span)? {
                            if let Some(call) = self.proof_points.ordinary_array_receivers.get(binding) {
                                self.aggregate_result.ordinary_array_incompatible_uses.insert(*call);
                            }
                        }
                    }
                }
                let guarded_length = matches!(read_mode, ExpressionReadMode::ArrayLengthArgument)
                    && *mutability == ScalarReferenceMutability::Shared
                    && matches!(place, ScalarPlace::Name { span, .. }
                        if matches!(self.prepared_place_root(root.ordinal, *span)?,
                            Some(PreparedPlaceRoot::Lexical(binding))
                                if self.proof_points.fallible_allocation_bindings.as_ref().and_then(|sites| sites.get(binding))
                                    .is_some_and(|site| site.has_runtime_extent())
                                    || self.proof_points.guarded_runtime_receivers.contains(binding)));
                if !guarded_length && !matches!(read_mode, ExpressionReadMode::FallibleAliasInitializer
                    | ExpressionReadMode::GuardedWholeReleaseArgument) {
                    self.reject_fallible_address(place, place_ordinal, span)?;
                }
                let proof_place = self.proof_place(place, place_ordinal);
                if let ScalarPlace::Index { base, index, .. } = proof_place.as_ref() {
                    if direct_checked_field_base(base).is_some()
                        && !matches!(index.as_ref(), ScalarExpression::Integer { .. })
                    {
                        return Err(self.missing_context(
                            span,
                            ReferenceAnalysisErrorReason::UnprovedCheckedArrayIndex,
                        ));
                    }
                }

                let mut named_parent_point = None;
                let mut reborrow = None;
                let parents = match root.place {
                    ScalarPlace::Dereference { pointer, .. } => {
                        let pointer_ordinal = self.expression_ordinal(
                            OccurrenceParent::Place(root.ordinal),
                            OccurrenceEdge::DereferencePointer,
                        );
                        let value = self.expression(pointer, scope, pointer_ordinal)?;
                        // A named checked-field literal index always goes through
                        // exact edge replay, including the one-known-parent case.
                        let indexed_projection = matches!(place, ScalarPlace::Index { base, .. }
                            if direct_checked_field_base(base).is_some());
                        let ordinary_origins = match value {
                            ReferenceValue::Checked(origins) if !indexed_projection => Some(origins),
                            _ => None,
                        };
                        let Some(origins) = ordinary_origins else {
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
                                    self.deferred_reborrow_roots.iter().find(|(handle, _, _, _)| *handle == binding)
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
                            let joined_mode = if !self.has_joined_root_reborrow {
                                Some(ScalarReferenceMutability::Mutable)
                            } else if self.proof_points.joined_reborrow_descendants.len() < MAX_JOINED_REBORROW_DESCENDANTS {
                                Some(if self.joined_mutable_child.as_ref() == Some(&binding) {
                                    ScalarReferenceMutability::Mutable
                                } else {
                                    ScalarReferenceMutability::Shared
                                })
                            } else {
                                None
                            };
                            let joined_source = self.binding_types.get(&binding).is_some_and(|ty| {
                                let ScalarType::CheckedReference { mutability: parent_mode, inner } = ty else { return false };
                                if Some(*parent_mode) != joined_mode { return false; }
                                if matches!(place, ScalarPlace::Dereference { .. }) {
                                    return aggregate::ordinary_value(inner);
                                }
                                if self.has_joined_root_reborrow {
                                    return false;
                                }
                                let checked_types = &self.call_sites.structure.checked_types;
                                let Some(field_type) = checked_types.places.get(&place_ordinal)
                                    .filter(|ty| aggregate::ordinary_value(ty)) else { return false; };
                                let (field_place, index) = joined_projection_field(proof_place.as_ref());
                                if !matches!(direct_checked_field_base(field_place),
                                    Some((written, ScalarFieldReference::Resolved(field)))
                                        if written == pointer.as_ref()
                                            && matches!(inner.as_ref(), ScalarType::Struct(id) if id == &field.structure))
                                {
                                    return false;
                                }
                                if let Some(ScalarExpression::Integer { value, .. }) = index {
                                    let field_occurrence = self.place_ordinal(
                                        OccurrenceParent::Place(place_ordinal), OccurrenceEdge::PlaceBase,
                                    );
                                    let Some(ScalarType::Array { element, length, .. }) = checked_types.places.get(&field_occurrence)
                                        else { return false; };
                                    if element.as_ref() != field_type || value < &0.into() || value >= &(*length).into() {
                                        return false;
                                    }
                                } else if index.is_some() {
                                    return false;
                                }
                                matches!(self.call_sites.prepared_plain_name(pointer_ordinal),
                                    Some(PlainNameIdentity::Lexical(expected)) if expected == &binding)
                                    && self.call_sites.structure.checked_types.expressions.get(&pointer_ordinal) == Some(ty)
                                    && matches!(self.call_sites.structure.checked_types.expressions.get(&ordinal),
                                        Some(ScalarType::CheckedReference { mutability: output_mode, inner })
                                            if output_mode == mutability && inner.as_ref() == field_type)
                            });
                            if (*mutability == ScalarReferenceMutability::Shared || !self.has_joined_root_reborrow
                                || self.joined_mutable_child.as_ref() == Some(&binding))
                                && joined_source && self.joined_local_reborrow
                                && matches!(binding.kind, ReferenceBindingKind::Declared)
                                && *self.joined_reborrow_call_admission.get_or_insert_with(|| {
                                    joined_reborrow_calls_are_supported(
                                        &self.source, self.call_sites, self.shared_scalar_read_effects,
                                    )
                                })
                            {
                                let pointer_point = self.cursor.expect("named checked read");
                                self.has_projected_child |= direct_checked_field_base(place).is_some();
                                let point = self.point_with_origins(
                                    CfgPointKind::ReborrowThrough { mode: *mutability, place: proof_place.clone().into_owned(), pointer_point },
                                    span, scope, None,
                                );
                                if self.has_joined_root_reborrow {
                                    self.proof_points.joined_reborrow_descendants.push(point);
                                } else {
                                    self.has_joined_root_reborrow = true;
                                }
                                self.aggregate_facts.receipt_loan_sinks.insert(point, vec![pointer_point]);
                                return Ok(ReferenceValue::Uncomputed);
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
                    collect_reference_projections(proof_place.as_ref(), parent_loan.as_ref(), &mut projections);
                    if self.widened_arrays.as_ref().is_some_and(|owners| owners.contains(&binding))
                        && !matches!(read_mode, ExpressionReadMode::ArrayLengthArgument)
                        && !matches!(read_mode, ExpressionReadMode::WidenedBorrowAddress(owner)
                            if owner == binding.declaration_span && parent_loan.is_none()
                                && projections.is_empty()
                                && matches!(proof_place.as_ref(), ScalarPlace::Name { .. })
                                && *mutability == ScalarReferenceMutability::Shared)
                        && !(parent_loan.is_none()
                            && self.exact_widened_element_borrow(place, place_ordinal, ordinal, &binding, *mutability))
                    {
                        return Err(self.missing_context(span, ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer));
                    }
                    let loan = ReferenceLoanId {
                        mode: *mutability,
                        origin_place: ReferencePlaceId {
                            source: self.source.clone(),
                            function_span: self.function_span,
                            declaration_span,
                            block_span: declaration_block,
                            binding,
                            projections,
                            place: proof_place.clone().into_owned(),
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
                        self.has_projected_child |= direct_checked_field_base(place).is_some();
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
                let operand = self.expression_ordinal(parent, OccurrenceEdge::DereferencePointer);
                self.expression_with_read_mode(pointer, scope, operand, ExpressionReadMode::DereferenceOperand)?;
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
            ScalarPlace::Index { base, index, span, .. } => {
                if let Some(binding) = root_binding {
                    self.record_indexed_holder(place, *span, binding, ordinal);
                }
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

    fn record_indexed_holder(
        &mut self, place: &ScalarPlace, span: ByteSpan, holder: &ReferenceBindingId,
        ordinal: OccurrenceOrdinal,
    ) {
        let ScalarPlace::Index { base, index, .. } = place else { return };
        if !matches!(base.as_ref(), ScalarPlace::Name { .. })
            || !matches!(index.as_ref(), ScalarExpression::Name { .. })
            || !matches!(self.binding_types.get(holder), Some(ScalarType::Array { element, .. })
                if matches!(element.as_ref(), ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner }
                    if aggregate::ordinary_value(inner)))
            || !self.call_sites.call_occurrences.is_empty()
            || !self.call_sites.callable_escapes.is_empty()
        {
            return;
        }
        let index = self.expression_ordinal(OccurrenceParent::Place(ordinal), OccurrenceEdge::PlaceIndex);
        let Some(PlainNameIdentity::Lexical(selector)) = self.call_sites.prepared_plain_name(index) else { return };
        if self.call_sites.structure.checked_types.expressions.get(&index) != Some(&ScalarType::U64)
            || !self.call_sites.boundary.parameters.iter().any(|slot|
                slot.ty == ScalarType::U64 && slot.span == selector.declaration_span)
            || self.call_sites.boundary.parameters.iter().any(|slot| slot.span == holder.declaration_span)
        {
            return;
        }
        self.proof_points.indexed_holders.get_or_insert_with(Default::default)
            .record(holder, selector, span);
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

    fn exact_scalar_receipt_dereference(&self, point: CfgPointId, ordinal: OccurrenceOrdinal) -> bool {
        let calls = self.call_sites;
        let CfgPointKind::ReadPlace { place: ScalarPlace::Dereference { .. },
            pointer_point: Some(input), reads_origin: false, .. } = &self.cfg.points[point.0].kind
        else { return false };
        let place = self.place_ordinal(OccurrenceParent::Expression(ordinal), OccurrenceEdge::ExpressionPlace);
        let pointer_ordinal = self.expression_ordinal(OccurrenceParent::Place(place), OccurrenceEdge::DereferencePointer);
        let Some(ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner }) =
            calls.structure.checked_types.expressions.get(&pointer_ordinal) else { return false };
        if !aggregate::ordinary_value(inner)
            || calls.structure.checked_types.expressions.get(&ordinal) != Some(inner.as_ref())
        {
            return false;
        }
        match &self.cfg.points[input.0].kind {
            CfgPointKind::Read { binding } => matches!(calls.prepared_plain_name(pointer_ordinal),
                Some(PlainNameIdentity::Lexical(expected)) if expected == binding),
            _ => false,
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
        if let Some(binding) = root_binding {
            let guarded_array = self.proof_points.fallible_allocation_bindings.as_ref().and_then(|sites| sites.get(binding))
                .filter(|owner| owner.has_indexed_extent());
            if let Some(owner) = guarded_array {
                let exact = match source {
                    CheckedReadSource::Place(ordinal) => self.exact_guarded_index(place, ordinal, binding, owner).is_some(),
                    _ => false,
                };
                if !exact {
                    return Err(self.missing_origin(span, binding.clone(),
                        ReferenceAnalysisErrorReason::UnsupportedFallibleAllocationAccess));
                }
            }
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
        let proof_place = match source {
            CheckedReadSource::Place(ordinal) => self.proof_place(place, ordinal),
            _ => std::borrow::Cow::Borrowed(place),
        };
        let candidates = match (source, root_binding) {
            (CheckedReadSource::PreparedMember { binding, .. }, _) | (_, Some(binding)) => {
                vec![self.reference_place(proof_place.as_ref(), binding.clone(), None)]
            }
            _ if pointer_point.is_none() => vec![self.assignment_place(proof_place.as_ref(), span)?],
            _ => Vec::new(),
        };
        let event = CfgPointKind::ReadPlace {
            place: proof_place.into_owned(),
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

    fn pure_widened_literal_count(binding: &ScalarBinding) -> Option<u64> {
        let (_, elements, _) = super::widened_local_literal(binding)?;
        elements.iter().all(aggregate::pure_widened_element)
            .then(|| u64::try_from(elements.len()).ok()).flatten()
    }

    fn scalar_aggregate_member(
        &mut self, value: &ScalarExpression, scope: ReferenceScopeId, ordinal: OccurrenceOrdinal,
        exact_type: Option<&ScalarType>, guarded: bool,
        ordinary: &mut Option<Vec<aggregate::GuardedNumericExpression>>,
    ) -> Result<(), ReferenceAnalysisError> {
        let candidate = (guarded || ordinary.is_some()) && matches!(value,
            ScalarExpression::Call { .. } | ScalarExpression::Binary { .. } | ScalarExpression::If { .. });
        let before = self.cursor;
        let previous = self.capture_call_points;
        self.capture_call_points |= candidate;
        let result = self.expression(value, scope, ordinal);
        self.capture_call_points = previous;
        result?;
        if candidate {
            let proof = exact_type.filter(|_| self.generic_parameters.is_empty())
                .and_then(|ty| self.guarded_numeric_expression(value, ordinal, ty));
            match (proof, guarded) {
                (Some(proof), true) => self.aggregate_result.fallible_call_expressions.push(proof),
                (Some(proof), false) => ordinary.as_mut().expect("candidate scalar aggregate").push(proof),
                (None, true) => return Err(self.missing_context(expression_span(value),
                    ReferenceAnalysisErrorReason::UnprovedFallibleCallInitializer)),
                (None, false) => *ordinary = None,
            }
        }
        let lexical_read = || self.cursor.is_some_and(|point| {
            Some(point) != before && matches!(self.cfg.points[point.0].kind, CfgPointKind::Read { .. })
        });
        if guarded && matches!(value, ScalarExpression::Name { .. }) && !lexical_read() {
            return Err(self.missing_context(expression_span(value),
                ReferenceAnalysisErrorReason::NonLexicalFallibleStructInitializer));
        }
        if ordinary.is_some() && (!exact_type.is_some_and(aggregate::ordinary_value)
            || (!candidate && !match value {
                ScalarExpression::Name { .. } => lexical_read(),
                ScalarExpression::Integer { .. } | ScalarExpression::Boolean { .. }
                | ScalarExpression::Char { .. } | ScalarExpression::Float { .. } => true,
                ScalarExpression::Unary { operator: UnaryOperator::Negate, operand, .. } =>
                    matches!(operand.as_ref(), ScalarExpression::Float { .. }),
                _ => false,
            }))
        {
            *ordinary = None;
        }
        Ok(())
    }

    fn guarded_numeric_expression(
        &self, value: &ScalarExpression, ordinal: OccurrenceOrdinal, payload: &ScalarType,
    ) -> Option<aggregate::GuardedNumericExpression> {
        Self::prove_guarded_numeric_expression(self.call_sites, &self.cfg, value, ordinal, payload)
    }

    fn prove_guarded_numeric_expression(
        sites: &FunctionCallSites, cfg: &ScalarReferenceCfg,
        value: &ScalarExpression, ordinal: OccurrenceOrdinal, payload: &ScalarType,
    ) -> Option<aggregate::GuardedNumericExpression> {
        if !matches!(payload, ScalarType::U64 | ScalarType::I32 | ScalarType::Bool) { return None; }
        let mut proof = aggregate::GuardedNumericExpression {
            expression: ordinal, calls: Vec::new(), operations: Vec::new(),
        };
        use aggregate::GuardedNumericNode as Node;
        let mut stack = vec![(value, ordinal, payload, false)];
        while let Some((value, occurrence, payload, children_proved)) = stack.pop() {
            if sites.structure.checked_types.expressions.get(&occurrence) != Some(payload) {
                return None;
            }
            match value {
                ScalarExpression::Integer { value, .. } =>
                    proof.operations.push(Node::Atom(u64::try_from(value.clone()).unwrap_or(u64::MAX))),
                ScalarExpression::Boolean { value, .. } if *payload == ScalarType::Bool =>
                    proof.operations.push(Node::Atom(u64::from(*value))),
                ScalarExpression::Name { .. } => {
                    if !matches!(sites.prepared_plain_name(occurrence), Some(PlainNameIdentity::Lexical(_))) {
                        return None;
                    }
                    proof.operations.push(Node::Atom(u64::MAX));
                }
                ScalarExpression::Call { arguments, type_arguments, .. } => {
                    let nested = arguments.iter().any(|argument| matches!(argument, ScalarExpression::Call { .. }));
                    if children_proved {
                        proof.operations.push(Node::Call(occurrence, arguments.len()));
                        continue;
                    }
                    let signature = sites.checked_call_signatures.get(&occurrence)?;
                    let point = *cfg.primary_call_points.get(&occurrence)?;
                    let CfgPointKind::Call { output: 0, argument_points,
                        target: ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. }, .. } =
                        &cfg.points.get(point.0)?.kind else { return None; };
                    if !type_arguments.is_empty() || sites.declared_unsafe_calls.contains(&occurrence.0)
                        || !signature.parameters.iter().all(aggregate::ordinary_value)
                        || !matches!(signature.outputs.outputs.as_slice(), [output] if &output.ty == payload)
                    { return None; }
                    proof.calls.push((point, occurrence));
                    if nested {
                        if *payload != ScalarType::U64 || signature.parameters.len() != arguments.len()
                            || !signature.parameters.iter().all(|ty| *ty == ScalarType::U64)
                            || argument_points.len() != arguments.len()
                        { return None; }
                        stack.push((value, occurrence, payload, true));
                        for (position, argument) in arguments.iter().enumerate().rev() {
                            let OccurrenceNode::Expression(child) = sites.structure.edges.get(&(
                                OccurrenceParent::Expression(occurrence), OccurrenceEdge::CallArgument(position),
                            ))? else { return None; };
                            match argument {
                                ScalarExpression::Call { .. }
                                    if sites.structure.checked_types.expressions.get(child) == Some(&ScalarType::U64)
                                        && cfg.primary_call_points.get(child) == Some(&argument_points[position]) => {}
                                ScalarExpression::Name { .. } | ScalarExpression::Integer { .. } => {}
                                _ => return None,
                            }
                            stack.push((argument, *child, payload, false));
                        }
                    } else {
                        if !arguments.iter().all(|argument| matches!(argument, ScalarExpression::Name { .. })
                            || aggregate::pure_widened_element(argument))
                        { return None; }
                        proof.operations.push(Node::Call(occurrence, 0));
                    }
                }
                ScalarExpression::Binary { operator, left, right, .. } => {
                    if children_proved {
                        proof.operations.push(match operator {
                            BinaryOperator::Add if *payload == ScalarType::U64 => Node::Add,
                            BinaryOperator::BitAnd if *payload != ScalarType::Bool => Node::And,
                            BinaryOperator::BitOr | BinaryOperator::BitXor if *payload != ScalarType::Bool => Node::OrXor,
                            BinaryOperator::And | BinaryOperator::Or if *payload == ScalarType::Bool => Node::Logical,
                            _ => return None,
                        });
                    } else {
                        let child = |edge| match sites.structure.edges.get(&(OccurrenceParent::Expression(occurrence), edge)) {
                            Some(OccurrenceNode::Expression(child)) => Some(*child),
                            _ => None,
                        };
                        stack.push((value, occurrence, payload, true));
                        let (left_site, right_site) = if matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
                            let region = sites.short_circuits.get(&occurrence)?;
                            (region.left, region.right)
                        } else {
                            (child(OccurrenceEdge::BinaryLeft)?, child(OccurrenceEdge::BinaryRight)?)
                        };
                        stack.push((right, right_site, payload, false));
                        stack.push((left, left_site, payload, false));
                    }
                }
                ScalarExpression::If { condition, then_branch, else_branch, .. } => {
                    if children_proved {
                        proof.operations.push(Node::Select);
                    } else {
                        let ConditionalRegion::Value { condition: predicate, then_block, else_block, .. } =
                            sites.conditionals.get(&occurrence)? else { return None; };
                        let arm = |block: &ScalarBlock, region: BlockOrdinal| {
                            let [ScalarBlockItem::Expression(_)] = block.items.as_slice() else { return None; };
                            let [BlockItem { kind: BlockItemKind::FinalOutput(row), .. }] =
                                sites.structure.blocks.get(region.0)?.items.as_slice() else { return None; };
                            (!block.unsafe_context && row.output == 0).then_some(row.expression)
                        };
                        let [ScalarBlockItem::Expression(left)] = then_branch.items.as_slice() else { return None; };
                        let [ScalarBlockItem::Expression(right)] = else_branch.items.as_slice() else { return None; };
                        stack.push((value, occurrence, payload, true));
                        stack.push((right, arm(else_branch, *else_block)?, payload, false));
                        stack.push((left, arm(then_branch, *then_block)?, payload, false));
                        stack.push((condition, *predicate, &ScalarType::Bool, false));
                    }
                }
                _ => return None,
            }
        }
        (!proof.calls.is_empty()).then_some(proof)
    }

    fn guarded_runtime_call_paths(
        &self, value: &ScalarExpression, ordinal: OccurrenceOrdinal, ty: &ScalarType,
    ) -> Option<Vec<(CfgPointId, OccurrenceOrdinal)>> {
        let sites = self.call_sites;
        let mut proven = Vec::new();
        let mut nodes = vec![(value, ordinal)];
        while let Some((value, occurrence)) = nodes.pop() {
            if !sites.structure.checked_types.expressions.get(&occurrence)
                .is_some_and(|actual| aggregate::same_runtime_array_type(actual, ty))
            { return None; }
            match value {
                ScalarExpression::Call { .. } => {
                    let point = *self.cfg.primary_call_points.get(&occurrence)?;
                    let signature = sites.checked_call_signatures.get(&occurrence)?;
                    if sites.declared_unsafe_calls.contains(&occurrence.0)
                        || signature.parameters.as_slice() != [ScalarType::U64]
                        || !matches!(signature.outputs.outputs.as_slice(), [output]
                            if aggregate::same_runtime_array_type(&output.ty, ty))
                        || !matches!(&self.cfg.points.get(point.0)?.kind, CfgPointKind::Call {
                            output: 0, target: ResolvedCallTarget::ModuleCallable {
                                concrete: ConcreteCallSelection::Declaration, ..
                            }, argument_points, ..
                        } if argument_points.len() == 1)
                    { return None; }
                    proven.push((point, occurrence));
                }
                ScalarExpression::If { condition, then_branch, else_branch, .. } => {
                    let ConditionalRegion::Value { condition: predicate, then_block, else_block, .. } =
                        sites.conditionals.get(&occurrence)? else { return None; };
                    if sites.structure.checked_types.expressions.get(predicate) != Some(&ScalarType::Bool)
                        || !match condition.as_ref() {
                            ScalarExpression::Boolean { .. } => true,
                            ScalarExpression::Name { .. } => matches!(sites.prepared_plain_name(*predicate),
                                Some(PlainNameIdentity::Lexical(_))),
                            ScalarExpression::Binary { operator: BinaryOperator::Equal | BinaryOperator::NotEqual,
                                left, right, .. } =>
                                [(left.as_ref(), OccurrenceEdge::BinaryLeft), (right.as_ref(), OccurrenceEdge::BinaryRight)]
                                    .into_iter().all(|(value, edge)| {
                                        let Some(OccurrenceNode::Expression(operand)) = sites.structure.edges
                                            .get(&(OccurrenceParent::Expression(*predicate), edge)) else { return false; };
                                        sites.structure.checked_types.expressions.get(operand) == Some(&ScalarType::U64)
                                            && match value {
                                                ScalarExpression::Integer { .. } => true,
                                                ScalarExpression::Name { .. } => matches!(sites.prepared_plain_name(*operand),
                                                    Some(PlainNameIdentity::Lexical(_))),
                                                _ => false,
                                            }
                                    }),
                            _ => false,
                        }
                    { return None; }
                    for (branch, block) in [(else_branch, else_block), (then_branch, then_block)] {
                        let [ScalarBlockItem::Expression(value)] = branch.items.as_slice() else { return None; };
                        let [BlockItem { kind: BlockItemKind::FinalOutput(row), .. }] =
                            sites.structure.blocks.get(block.0)?.items.as_slice() else { return None; };
                        if branch.unsafe_context || row.output != 0 { return None; }
                        nodes.push((value, row.expression));
                    }
                }
                _ => return None,
            }
        }
        (!proven.is_empty()).then_some(proven)
    }

    fn widened_join_inputs(
        &self,
        value: &ScalarExpression,
        destination: &ScalarType,
        occurrence: OccurrenceOrdinal,
        scope: ReferenceScopeId,
    ) -> Option<[(usize, ByteSpan); 2]> {
        if scope != ReferenceScopeId(0) || !self.generic_parameters.is_empty()
            || !matches!(destination, ScalarType::RuntimeArray { .. })
        { return None; }
        let ScalarExpression::If { then_branch, else_branch, .. } = value else { return None; };
        let ConditionalRegion::Value { then_block, else_block, .. } =
            self.call_sites.conditionals.get(&occurrence)? else { return None };
        let arm = |block: &ScalarBlock, ordinal: BlockOrdinal| {
            let [ScalarBlockItem::LocalBinding(local), ScalarBlockItem::Expression(ScalarExpression::Name { name, .. })] =
                block.items.as_slice() else { return None };
            let [local_receiver] = local.receivers.as_slice() else { return None };
            if block.unsafe_context || &local_receiver.name != name || self.names.contains_key(name)
                || !aggregate::same_runtime_array_type(&local_receiver.ty, destination)
            { return None; }
            // The arm has no effect on an enclosing owner.
            Self::pure_widened_literal_count(local)?;
            let [_, BlockItem { kind: BlockItemKind::FinalOutput(output), .. }] =
                self.call_sites.structure.blocks[ordinal.0].items.as_slice() else { return None };
            (output.output == 0).then_some((output.expression.0, local_receiver.name_span))
        };
        Some([arm(then_branch, *then_block)?, arm(else_branch, *else_block)?])
    }

    fn widened_owning_argument(
        &self, value: &ScalarExpression, call: OccurrenceOrdinal, block: ByteSpan,
    ) -> Option<&ReferenceBindingId> {
        let owners = self.widened_arrays.as_ref()?;
        let calls = self.call_sites;
        let ScalarExpression::Call { arguments, .. } = value else { return None };
        let [ScalarExpression::Name { span, .. }] = arguments.as_slice() else { return None };
        if !self.generic_parameters.is_empty() || calls.declared_unsafe_calls.contains(&call.0)
            || !matches!(self.call_target(value, call), Ok(ResolvedCallTarget::ModuleCallable {
                concrete: ConcreteCallSelection::Declaration, ..
            }))
        { return None; }
        let signature = calls.checked_call_signatures.get(&call)?;
        if !aggregate::neutral_array_signature(&signature.parameters, &signature.outputs) { return None; }
        let OccurrenceNode::Expression(argument) = calls.structure.edges.get(&(
            OccurrenceParent::Expression(call), OccurrenceEdge::CallArgument(0)))? else { return None };
        let PlainNameIdentity::Lexical(source) = self.prepared_name_identity(*argument, *span).ok().flatten()?
        else { return None; };
        (owners.contains(source) && source.block_span == block).then_some(source)
    }

    fn exact_widened_binding_move(
        &self,
        binding: &ScalarBinding,
        value: &ScalarExpression,
        ordinal: OccurrenceOrdinal,
        block: ByteSpan,
    ) -> bool {
        let Some(owners) = &self.widened_arrays else { return false };
        let ScalarExpression::Name { span, .. } = value else { return false };
        let [receiver] = binding.receivers.as_slice() else { return false };
        let Ok(Some(PlainNameIdentity::Lexical(source))) = self.prepared_name_identity(ordinal, *span)
        else { return false };
        !binding.is_allocation && binding.fallible_assignment.is_none()
            && source.block_span == block && owners.contains(source)
            && self.binding_types.get(source).is_some_and(|ty| aggregate::same_runtime_array_type(ty, &receiver.ty))
    }

    fn move_value(
        &mut self,
        value: &ScalarExpression,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
    ) -> Result<(), ReferenceAnalysisError> {
        self.move_value_with_widened_receipt(value, scope, ordinal, false)
    }

    fn move_value_with_widened_receipt(
        &mut self,
        value: &ScalarExpression,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
        exact_widened_destination: bool,
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
        if !exact_widened_destination && self.widened_arrays.as_ref().is_some_and(|owners| owners.contains(binding)) {
            return Err(self.missing_context(span, ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer));
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
                        let projection = self.project_call_output(producer, primary, *output);
                        if self.aggregate_result.multi_result_bindings.contains_key(&producer) {
                            self.aggregate_facts.multi_owner_projections.insert(projection, (producer, *output));
                        }
                        if *output == 1 && self.aggregate_facts.secondary_owner_calls.contains(&producer)
                            && self.aggregate_result.mixed_result_bindings.contains(&producer)
                        {
                            self.aggregate_facts.secondary_owner_outputs.insert(projection, producer);
                        }
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
            self.binding_types.record(id.clone(), receiver.ty.clone(), self.deferred_local_reborrow.then_some(scope));
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
            || !self.binding_types.is_local_reborrow_owner(owner, source.scope, self.cfg.points[bind.0].scope)
            || !place.projections.is_empty()
            || !matches!(&place.place, ScalarPlace::Name { name, .. } if self.names.get(name) == Some(owner))
            || !matches!(&self.cfg.points[bind.0].kind, CfgPointKind::Bind { binding: actual } if actual == binding)
            || !matches!(self.cfg.facts.get(origin.0), Some(ReferenceOrigin::Fresh { loan: fact, .. }) if fact == loan)
        {
            return None;
        }
        Some((super::control::DeferredReborrowRoot::Local { owner: owner.clone(), borrow: value }, origin))
    }

    fn joined_shared_alias_source(&self, position: BlockPosition, read: CfgPointId, ty: &ScalarType) -> bool {
        let structure = &self.call_sites.structure;
        let Some(BlockItem { kind: BlockItemKind::Binding(binding), .. }) = structure.blocks.get(position.block.0)
            .and_then(|block| block.items.get(position.item)) else { return false };
        let BindingInitialization::Outputs(init) = &binding.initialization else { return false };
        let CfgPointKind::Read { binding: source } = &self.cfg.points[read.0].kind else { return false };
        binding.receivers().len() == 1 && init.secondary.is_empty()
            && structure.checked_types.expressions.get(&init.primary) == Some(ty)
            && matches!(self.call_sites.prepared_plain_name(init.primary),
                Some(PlainNameIdentity::Lexical(prepared)) if prepared == source)
    }

    fn borrowed_extent_argument(
        &self,
        argument: CfgPointId,
        aliases: usize,
    ) -> Option<(ByteSpan, Option<ByteSpan>)> {
        let points = &self.cfg.points;
        let mut value = argument;
        let mut alias = None;
        let installs = &self.proof_points.checked_binding_initializers;
        for &(bind, initial) in installs.get(installs.len().checked_sub(aliases)?..)?.iter().rev() {
            let CfgPointKind::Read { binding } = &points.get(value.0)?.kind else { return None };
            if !matches!(self.binding_types.get(binding), Some(ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Shared, inner,
            }) if matches!(inner.as_ref(), ScalarType::RuntimeArray { .. }))
                || !matches!(&points[bind.0].kind, CfgPointKind::Bind { binding: actual } if actual == binding)
                || points[bind.0].scope != ReferenceScopeId(0)
                || points[bind.0].successors.as_slice() != [value]
                || points[initial.0].successors.as_slice() != [bind]
            {
                return None;
            }
            alias.get_or_insert(binding.declaration_span);
            value = initial;
        }
        let CfgPointKind::Borrow { place, loan } = &points.get(value.0)?.kind else { return None };
        (loan.parent.is_none() && loan.mode == ScalarReferenceMutability::Shared
            && place.projections.is_empty() && matches!(place.place, ScalarPlace::Name { .. })
            && self.cfg.allocations.contains(&place.binding))
            .then_some((place.binding.declaration_span, alias))
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
                if let Some(author) = &mut self.local_borrow {
                    if allocation || (matches!(ty, ScalarType::CheckedReference { .. })
                        && (receiver != 0 || !author.bind(binding, bind, initializer)))
                    {
                        self.local_borrow = None;
                    }
                }
                if !allocation && matches!(ty, ScalarType::CheckedReference { .. }) {
                    if let Some(value) = initializer {
                        let frozen = &self.cfg.points[bind.0].possible_origins;
                        if frozen.is_none()
                            || frozen == &self.cfg.points[value.0].possible_origins
                        {
                            self.proof_points.checked_binding_initializers.push((bind, value));
                            if self.has_joined_root_reborrow && receiver == 0
                                && self.joined_mutable_child.is_none()
                                && matches!(ty, ScalarType::CheckedReference {
                                    mutability: ScalarReferenceMutability::Mutable, inner,
                                } if aggregate::ordinary_value(inner))
                                && matches!(self.cfg.points[value.0].kind,
                                    CfgPointKind::ReborrowThrough { mode: ScalarReferenceMutability::Mutable, .. })
                                && self.cfg.points[value.0].successors.as_slice() == [bind]
                                && self.cfg.points[value.0].scope == self.cfg.points[bind.0].scope
                            {
                                self.joined_mutable_child = Some(binding.clone());
                            }
                            if self.has_joined_root_reborrow && receiver == 0
                                && matches!(ty, ScalarType::CheckedReference {
                                    mutability: ScalarReferenceMutability::Shared, inner,
                                } if aggregate::ordinary_value(inner))
                                && self.joined_shared_alias_source(position, value, ty)
                            {
                                self.proof_points.joined_shared_aliases.push((bind, value));
                            }
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

    fn returned_array_index_selector_syntax(index: &ScalarExpression) -> bool {
        match index {
            ScalarExpression::Integer { .. } => true,
            ScalarExpression::Binary { operator: BinaryOperator::BitAnd, left, right, .. } =>
                matches!(left.as_ref(), ScalarExpression::Integer { .. })
                    || matches!(right.as_ref(), ScalarExpression::Integer { .. }),
            _ => false,
        }
    }

    fn names_literal_array_owner_at_index(&self, read: CfgPointId) -> bool {
        matches!(&self.cfg.points[read.0].kind, CfgPointKind::ReadPlace {
            pointer_point: None, candidates, reads_origin: false, ..
        } if matches!(candidates.as_slice(), [candidate]
            if self.aggregate_facts.allocations.get(&candidate.binding).copied()
                .is_some_and(|bind| self.aggregate_facts.literal_array_bind.is_some_and(|allocation| allocation.bind == bind))
                && matches!(candidate.projections.as_slice(), [ReferencePlaceProjection::Index(_)])))
    }

    fn returned_array_index_observation(
        &self,
        expression: &ScalarExpression,
        output: OccurrenceOrdinal,
        read: CfgPointId,
    ) -> Option<aggregate::ReturnedArrayIndex> {
        let ScalarExpression::IndexedRead { place: ScalarPlace::Index { base, index, .. }, .. } = expression
        else { return None };
        if !matches!(base.as_ref(), ScalarPlace::Name { .. }) {
            return None;
        }
        let (first, upper_bound) = match index.as_ref() {
            ScalarExpression::Integer { value, .. } => {
                let first = CfgPointId(read.0.checked_sub(1)?);
                if !matches!(&self.cfg.points.get(first.0)?.kind,
                    CfgPointKind::Evaluate(ScalarExpression::Integer { value: actual, .. }) if actual == value)
                {
                    return None;
                }
                (first, u64::try_from(value.clone()).ok()?)
            }
            ScalarExpression::Binary { operator: BinaryOperator::BitAnd, left, right, .. } => {
                use OccurrenceEdge::*;
                let place = self.place_ordinal(OccurrenceParent::Expression(output), ExpressionPlace);
                let selector = self.expression_ordinal(OccurrenceParent::Place(place), PlaceIndex);
                let mask = match (left.as_ref(), right.as_ref()) {
                    (ScalarExpression::Integer { value, .. }, _)
                    | (_, ScalarExpression::Integer { value, .. }) => u64::try_from(value.clone()).ok()?,
                    _ => return None,
                };
                // The final mask bounds the computed selector for offset
                // representability, not for the actual installed array extent.
                // Pair every operand with its already-checked CFG event in
                // reverse evaluation order. No calls or storage projections
                // are admitted here.
                let mut next = read.0;
                let mut pending = vec![(selector, index.as_ref())];
                while let Some((ordinal, operand)) = pending.pop() {
                    if self.call_sites.structure.checked_types.expressions.get(&ordinal) != Some(&ScalarType::U64) {
                        return None;
                    }
                    next = next.checked_sub(1)?;
                    let point = self.cfg.points.get(next)?;
                    match operand {
                        ScalarExpression::Name { .. } => {
                            let formal = self.literal_array_formal_binding(ordinal, &ScalarType::U64)?;
                            if !matches!(&point.kind, CfgPointKind::Read { binding } if binding == formal) {
                                return None;
                            }
                        }
                        ScalarExpression::Integer { .. } => {
                            if !matches!(&point.kind, CfgPointKind::Evaluate(actual) if actual == operand) {
                                return None;
                            }
                        }
                        ScalarExpression::Binary { operator: operator @ (BinaryOperator::Add
                            | BinaryOperator::Subtract | BinaryOperator::Multiply | BinaryOperator::BitAnd
                            | BinaryOperator::BitOr | BinaryOperator::BitXor), left, right, span } => {
                            if !matches!(&point.kind, CfgPointKind::Evaluate(ScalarExpression::Binary {
                                operator: actual_operator, span: actual_span, ..
                            }) if operator == actual_operator && span == actual_span)
                            {
                                return None;
                            }
                            let parent = OccurrenceParent::Expression(ordinal);
                            pending.push((self.expression_ordinal(parent, BinaryLeft), left.as_ref()));
                            pending.push((self.expression_ordinal(parent, BinaryRight), right.as_ref()));
                        }
                        _ => return None,
                    }
                }
                (CfgPointId(next), mask)
            }
            _ => return None,
        };
        Some(aggregate::ReturnedArrayIndex { first, read, output, upper_bound })
    }

    fn observe_literal_array_bind(
        &mut self,
        position: BlockPosition,
        scope: ReferenceScopeId,
        binding: &ScalarBinding,
        initialization: &BindingInitialization,
        initializer: Option<CfgPointId>,
    ) {
        if scope != ReferenceScopeId(0) || position.item != 0
            || !matches!(self.call_sites.boundary.parameters.as_slice(),
                [] | [ParameterSlot { ordinal: 0, .. }])
            || !self.call_sites.boundary.parameters.iter().all(|parameter| aggregate::ordinary_value(&parameter.ty))
            || !matches!(self.call_sites.named_place_roots, NamedPlaceRootFacts::Prepared(_))
        {
            return;
        }
        let BindingInitialization::Allocation { lengths } = initialization else { return };
        let [length] = lengths.as_slice() else { return };
        let [receiver] = binding.receivers.as_slice() else { return };
        if length.position != 0
            || !matches!(receiver.ty, ScalarType::RuntimeArray { .. })
            || !matches!(self.call_sites.structure.checked_types.expressions.get(&length.expression),
                Some(ScalarType::U64))
        {
            return;
        }
        let Some(length_expression) = receiver.allocation_length.as_ref() else { return };
        let extent = match length_expression {
            ScalarExpression::Integer { value, .. } => {
                let Ok(elements) = u64::try_from(value.clone()) else { return };
                aggregate::FreshArrayExtent::Literal(elements)
            }
            ScalarExpression::Name { .. } if matches!(self.call_sites.boundary.root_outputs.as_slice(),
                [RootOutputType { output: 0, ty: ScalarType::RuntimeArray { .. } }]) =>
            {
                if self.literal_array_formal_binding(length.expression, &ScalarType::U64).is_none() {
                    return;
                }
                aggregate::FreshArrayExtent::Formal
            }
            _ => return,
        };
        let (Some(bind), Some(placeholder)) = (self.cursor, initializer) else { return };
        if !self.aggregate_facts.allocation_slots.contains_key(&bind) {
            return;
        }
        self.aggregate_facts.literal_array_bind = Some(aggregate::LiteralArrayBind {
            bind,
            placeholder,
            position,
            length_occurrence: length.expression,
            extent,
        });
    }

    fn literal_array_formal_binding(
        &self,
        occurrence: OccurrenceOrdinal,
        ty: &ScalarType,
    ) -> Option<&ReferenceBindingId> {
        let [formal] = self.call_sites.boundary.parameters.as_slice() else { return None };
        if formal.ordinal != 0 || &formal.ty != ty
            || self.call_sites.structure.checked_types.expressions.get(&occurrence) != Some(ty)
        {
            return None;
        }
        let span = self.call_sites.occurrences.get(occurrence.0)?.span;
        let PlainNameIdentity::Lexical(binding) =
            self.prepared_name_identity(occurrence, span).ok().flatten()?
        else { return None };
        let structure = &self.call_sites.structure;
        let formal = parameter_binding_identity(
            &self.source, self.function_span, structure.blocks[structure.root.0].span, formal,
        );
        (binding == &formal).then_some(binding)
    }

    /// A scalar operand enters the private route only as the sole checked
    /// CallArgument(0) occurrence and exact read of its caller formal.
    fn literal_array_argument(
        &self,
        producer: OccurrenceOrdinal,
        rhs: CfgPointId,
    ) -> Option<Option<aggregate::LiteralArrayArgument>> {
        let CfgPointKind::Call { output: 0, argument_points, .. } = &self.cfg.points.get(rhs.0)?.kind
        else { return None };
        match (self.call_sites.boundary.parameters.as_slice(), argument_points.as_slice()) {
            ([], []) => Some(None),
            ([formal], [read]) if formal.ordinal == 0 && aggregate::ordinary_value(&formal.ty) => {
                let structure = &self.call_sites.structure;
                let OccurrenceNode::Expression(argument) = structure.edges.get(&(
                    OccurrenceParent::Expression(producer), OccurrenceEdge::CallArgument(0),
                ))? else { return None };
                let point = self.cfg.points.get(read.0)?;
                let CfgPointKind::Read { binding } = &point.kind else { return None };
                (self.literal_array_formal_binding(*argument, &formal.ty) == Some(binding))
                    .then_some(Some(aggregate::LiteralArrayArgument {
                        read: *read, occurrence: *argument,
                    }))
            }
            _ => None,
        }
    }

    fn record_frozen_install(&mut self, destination: OccurrenceOrdinal, emitted: FrozenInstall) {
        if self.capture_call_points {
            self.proof_points.installs.insert(destination, emitted);
        }
    }

    fn emit_direct_assignment(
        &mut self,
        binding: ReferenceBindingId,
        assign: CfgPointKind,
        rhs: CfgPointId,
        value: ReferenceValue,
        span: ByteSpan,
        scope: ReferenceScopeId,
    ) -> Result<FrozenInstall, ReferenceAnalysisError> {
        let possible_origins = match value {
            ReferenceValue::Checked(origins) => {
                self.checked.insert(binding.clone());
                self.origins.insert(binding, origins.clone());
                Some(origins)
            }
            ReferenceValue::Uncomputed if self.checked.contains(&binding) => {
                self.origins.remove(&binding);
                None
            }
            _ if self.checked.contains(&binding) => {
                return Err(self.missing_origin(
                    span,
                    binding,
                    ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                ));
            }
            _ => {
                self.origins.remove(&binding);
                Some(BTreeSet::new())
            }
        };
        let install = self.point_with_origins(assign, span, scope, possible_origins);
        Ok(FrozenInstall { rhs, install })
    }

    fn author_direct_assignment(&mut self, emitted: FrozenInstall, prepared_existing: bool) {
        let Some(author) = &mut self.local_borrow else { return };
        let CfgPointKind::Assign { target, .. } = &self.cfg.points[emitted.install.0].kind
            else { unreachable!("direct assignment install") };
        if !prepared_existing || !match self.binding_types.get(&target.binding) {
            Some(ScalarType::CheckedReference { inner, .. }) if aggregate::ordinary_value(inner) =>
                author.assign(&target.binding, emitted.install, emitted.rhs),
            Some(ty) if aggregate::ordinary_value(ty) => author.accept_ordinary_store(),
            _ => false,
        } {
            self.local_borrow = None;
        }
    }

    fn exact_widened_element_borrow(
        &self, place: &ScalarPlace, ordinal: OccurrenceOrdinal, address: OccurrenceOrdinal,
        owner: &ReferenceBindingId, mutability: ScalarReferenceMutability,
    ) -> bool {
        let ScalarPlace::Index { base, index, .. } = place else { return false };
        let ScalarPlace::Name { span, .. } = base.as_ref() else { return false };
        let Some(ty @ ScalarType::RuntimeArray { element, .. }) = self.binding_types.get(owner)
            else { return false };
        let parent = OccurrenceParent::Place(ordinal);
        let base = self.place_ordinal(parent, OccurrenceEdge::PlaceBase);
        let selector = self.expression_ordinal(parent, OccurrenceEdge::PlaceIndex);
        let types = &self.call_sites.structure.checked_types;
        // Only non-mutating selectors can retain this exact backing while
        // indexing. Ownership and last-use still come from ordinary loan replay.
        aggregate::ordinary_value(element)
            && matches!(self.prepared_place_root(base, *span), Ok(Some(PreparedPlaceRoot::Lexical(binding))) if binding == owner)
            && types.places.get(&base) == Some(ty)
            && types.places.get(&ordinal) == Some(element.as_ref())
            && types.expressions.get(&selector) == Some(&ScalarType::U64)
            && matches!(types.expressions.get(&address), Some(ScalarType::CheckedReference {
                mutability: actual, inner,
            }) if *actual == mutability && inner == element)
            && match index.as_ref() {
                ScalarExpression::Integer { .. } => true,
                ScalarExpression::Name { .. } => matches!(self.call_sites.prepared_plain_name(selector),
                    Some(PlainNameIdentity::Lexical(_))),
                _ => false,
            }
    }

    fn exact_guarded_index<'b>(
        &self, place: &ScalarPlace, ordinal: OccurrenceOrdinal, owner: &ReferenceBindingId,
        site: &'b super::control::GuardedAllocationBinding,
    ) -> Option<&'b ScalarType> {
        if !site.has_indexed_extent() { return None; }
        let length = site.indexed_extent();
        let element = match &site.ty {
            ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => element,
            _ => return None,
        };
        let ScalarPlace::Index { base, index, .. } = place else { return None; };
        let ScalarPlace::Name { span, .. } = base.as_ref() else { return None; };
        let parent = OccurrenceParent::Place(ordinal);
        let base = self.place_ordinal(parent, OccurrenceEdge::PlaceBase);
        let selector = self.expression_ordinal(parent, OccurrenceEdge::PlaceIndex);
        let types = &self.call_sites.structure.checked_types;
        (matches!(self.prepared_place_root(base, *span), Ok(Some(PreparedPlaceRoot::Lexical(binding))) if binding == owner)
            && types.places.get(&base) == Some(&site.ty)
            && types.places.get(&ordinal) == Some(element.as_ref())
            && types.expressions.get(&selector) == Some(&ScalarType::U64)
            && match index.as_ref() {
                ScalarExpression::Integer { value, .. } =>
                    u64::try_from(value.clone()).ok().is_some_and(|index| length.is_none_or(|length| index < length)),
                _ => true,
            }).then_some(element.as_ref())
    }

    fn exact_indexed_store_type(
        &self, place: &ScalarPlace, destination: OccurrenceOrdinal, parent_type: &ScalarType, value_type: &ScalarType,
    ) -> bool {
        let normalized = self.proof_place(place, destination);
        let ScalarPlace::Index { base, index, .. } = normalized.as_ref() else { return false };
        let ScalarExpression::Integer { value, .. } = index.as_ref() else { return false };
        if !matches!(direct_checked_field_base(base),
            Some((ScalarExpression::Name { .. }, ScalarFieldReference::Resolved(field)))
                if matches!(parent_type, ScalarType::Struct(id) if id == &field.structure))
        {
            return false;
        }
        let structure = &self.call_sites.structure;
        let field = self.place_ordinal(OccurrenceParent::Place(destination), OccurrenceEdge::PlaceBase);
        let index = self.expression_ordinal(OccurrenceParent::Place(destination), OccurrenceEdge::PlaceIndex);
        let Some(ScalarType::Array { element, length, .. }) = structure.checked_types.places.get(&field)
            else { return false };
        aggregate::ordinary_value(value_type) && element.as_ref() == value_type
            && structure.checked_types.places.get(&destination) == Some(value_type)
            && structure.checked_types.expressions.get(&index) == Some(&ScalarType::U64)
            && value >= &0.into() && value < &(*length).into()
    }

    fn assignment_destination(
        &mut self,
        target: &ScalarAssignmentTarget,
        scope: ReferenceScopeId,
        ordinal: OccurrenceOrdinal,
    ) -> Result<ReferenceAssignmentDestination<'a>, ReferenceAnalysisError> {
        if let ScalarPlace::Name { span, .. } = &target.place {
            if let Some(DirectDestinationIdentity::Existing(binding)) = self.call_sites.prepared_direct_destination(ordinal) {
                if let Some(call) = self.proof_points.ordinary_array_receivers.get(binding) {
                    self.aggregate_result.ordinary_array_incompatible_uses.insert(*call);
                }
            }
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
                    self.reference_place(self.proof_place(&target.place, ordinal).as_ref(), binding, None),
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
        let scope_entry = self.point(
            CfgPointKind::ScopeEnter,
            region.span,
            scope,
            BTreeSet::new(),
        );
        let ordinary_root_eligible = scope == ReferenceScopeId(0) && ordinal == calls.structure.root
            && !block.unsafe_context && self.generic_parameters.is_empty()
            && calls.boundary.parameters.iter().all(|slot| aggregate::ordinary_value(&slot.ty))
            && calls.boundary.root_outputs.iter().all(|slot| aggregate::ordinary_value(&slot.ty));
        let mut declares_guarded_owner = false;
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
                    let widened_join = outputs.secondary.is_empty() && !binding.is_allocation
                        && binding.fallible_assignment.is_none() && binding_region.receivers().len() == 1
                        && binding.output_origin == ScalarBindingOutputOrigin::SingleExpression
                        && self.widened_join_inputs(primary, &receiver.ty, primary_ordinal, scope).is_some_and(|arms| {
                            self.aggregate_result.widened_join_arms.extend(arms);
                            self.aggregate_result.widened_join_receivers.insert((receiver.span.start, receiver.span.end));
                            true
                        });
                    let alias_owner = self.prepared_fallible_alias(binding, primary, primary_ordinal, scope, &receiver.ty);
                    let widened_receiver_source = if !binding.is_allocation && binding.fallible_assignment.is_none()
                        && binding_region.receivers().len() == 1 && outputs.secondary.is_empty()
                        && binding.output_origin == ScalarBindingOutputOrigin::SingleExpression
                        && !self.names.contains_key(&receiver.name)
                    {
                        self.widened_owning_argument(primary, primary_ordinal, block.span)
                            .filter(|source| self.binding_types.get(source).is_some_and(|ty|
                                aggregate::same_runtime_array_type(ty, &receiver.ty)))
                            .map(|source| source.declaration_span)
                    } else { None };
                    let fixed_copy_receiver = (|| {
                        if binding.is_allocation || binding.fallible_assignment.is_some()
                            || binding_region.receivers().len() != 1 || !outputs.secondary.is_empty()
                            || binding.output_origin != ScalarBindingOutputOrigin::SingleExpression
                            || !self.generic_parameters.is_empty() || self.names.contains_key(&receiver.name)
                            || calls.declared_unsafe_calls.contains(&primary_ordinal.0)
                        { return None; }
                        let ScalarExpression::Call { arguments, .. } = primary else { return None; };
                        let [ScalarExpression::Name { span, .. }] = arguments.as_slice() else { return None; };
                        let signature = calls.checked_call_signatures.get(&primary_ordinal)?;
                        if !aggregate::neutral_array_signature(&signature.parameters, &signature.outputs)
                            || !aggregate::same_runtime_array_type(&signature.parameters[0], &receiver.ty)
                            || !matches!(self.call_target(primary, primary_ordinal),
                                Ok(ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. }))
                        { return None; }
                        let OccurrenceNode::Expression(argument) = calls.structure.edges.get(&(
                            OccurrenceParent::Expression(primary_ordinal), OccurrenceEdge::CallArgument(0)))? else { return None; };
                        let PlainNameIdentity::Lexical(source) = self.prepared_name_identity(*argument, *span).ok().flatten()?
                        else { return None; };
                        let (_, count) = self.binding_types.get(source).and_then(|ty|
                            super::fixed_copy_to_runtime(&signature.parameters[0], ty))?;
                        Some((source.declaration_span, count))
                    })();
                    // Count expressions have their own prepared occurrence edge.
                    // Evaluate it before the allocation placeholder and owner
                    // Bind; the same ordered scalar proof is used for ordinary RHS.
                    let checked_count = (|| {
                        if block.unsafe_context || !self.generic_parameters.is_empty()
                            || !matches!(&receiver.ty, ScalarType::RuntimeArray { element, .. } if aggregate::ordinary_value(element))
                        { return None; }
                        let BindingInitialization::Allocation { lengths } = &binding_region.initialization else { return None; };
                        let [length] = lengths.as_slice() else { return None; };
                        let value @ (ScalarExpression::Call { .. } | ScalarExpression::Binary { .. } | ScalarExpression::If { .. }) =
                            binding.receivers[0].allocation_length.as_ref()? else { return None; };
                        let exact_call = !matches!(value, ScalarExpression::Call { .. }) || matches!(
                            calls.allocation_length_call_targets.get(&length.expression),
                            Some(ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. }));
                        (length.position == 0 && exact_call
                            && calls.structure.checked_types.expressions.get(&length.expression) == Some(&ScalarType::U64))
                            .then_some((value, length.expression))
                    })();
                    if let Some((value, length)) = checked_count {
                        let previous = self.capture_call_points;
                        self.capture_call_points = true;
                        let evaluated = self.expression(value, scope, length);
                        self.capture_call_points = previous;
                        evaluated?;
                        if let Some(proof) = self.guarded_numeric_expression(value, length, &ScalarType::U64) {
                            self.aggregate_result.ordinary_fallible_expressions.push(vec![proof]);
                        }
                    }
                    let before = self.cursor;
                    // SystemPanic terminates execution, not the current
                    // lexical scope via an unwind. Existing local owners
                    // retain their normal lexical release on success.
                    let ordinary_root_binding = ordinary_root_eligible
                        && binding.fallible_assignment.is_none() && !binding.is_allocation
                        && matches!(receiver.ty, ScalarType::U64 | ScalarType::I32)
                        && binding_region.receivers().len() == 1 && outputs.secondary.is_empty()
                        && binding.output_origin == ScalarBindingOutputOrigin::SingleExpression
                        && matches!(primary, ScalarExpression::Call { .. } | ScalarExpression::If { .. } | ScalarExpression::Binary { .. })
                        && calls.structure.edges.get(&(
                            OccurrenceParent::Block(calls.structure.root),
                            OccurrenceEdge::BindingRoot { item: position.item, output: 0 },
                        )) == Some(&OccurrenceNode::Expression(primary_ordinal));
                    let ordinary_aggregate = !block.unsafe_context && self.generic_parameters.is_empty()
                        && binding.fallible_assignment.is_none() && !binding.is_allocation
                        && binding_region.receivers().len() == 1 && outputs.secondary.is_empty()
                        && binding.output_origin == ScalarBindingOutputOrigin::SingleExpression
                        && (matches!((&receiver.ty, primary),
                            (ScalarType::Struct(_), ScalarExpression::StructLiteral { .. })
                            | (ScalarType::Array { .. }, ScalarExpression::ArrayLiteral { .. }))
                            || super::widened_local_literal(binding).is_some_and(|(_, _, _)|
                                calls.structure.checked_types.expressions.get(&primary_ordinal)
                                    .and_then(|actual| super::fixed_copy_to_runtime(&receiver.ty, actual)).is_some()))
                        && calls.structure.edges.get(&(
                            OccurrenceParent::Block(ordinal),
                            OccurrenceEdge::BindingRoot { item: position.item, output: 0 },
                        )) == Some(&OccurrenceNode::Expression(primary_ordinal));
                    let runtime_returned = binding.fallible_assignment.is_some()
                        && matches!(receiver.ty, ScalarType::RuntimeArray { .. })
                        && super::typecheck::guarded_runtime_rhs(primary);
                    let ordinary_runtime_receive = !block.unsafe_context && self.generic_parameters.is_empty()
                        && !binding.is_allocation && binding.fallible_assignment.is_none()
                        && binding_region.receivers().len() == 1 && outputs.secondary.is_empty()
                        && binding.output_origin == ScalarBindingOutputOrigin::SingleExpression
                        && matches!(&receiver.ty, ScalarType::RuntimeArray { element, .. } if aggregate::ordinary_value(element))
                        && super::typecheck::guarded_runtime_rhs(primary);
                    let walk_fallible_expression = !runtime_returned && !ordinary_runtime_receive
                        && (binding.fallible_assignment.is_some() || ordinary_root_binding)
                        && (matches!(primary, ScalarExpression::If { .. } | ScalarExpression::Binary { .. })
                            || matches!(primary, ScalarExpression::Call { arguments, .. }
                                if receiver.ty == ScalarType::Bool || arguments.iter().any(|argument| matches!(argument, ScalarExpression::Call { .. }))));
                    let value = if let Some((owner, count)) = fixed_copy_receiver {
                        self.expression_with_read_mode(primary, scope, primary_ordinal, ExpressionReadMode::FixedCopyOwningArgument(owner, count))?
                    } else if let Some(owner) = widened_receiver_source {
                        self.expression_with_read_mode(primary, scope, primary_ordinal, ExpressionReadMode::WidenedOwningRoundtrip(owner))?
                    } else if alias_owner.is_some() {
                        self.expression_with_read_mode(primary, scope, primary_ordinal, ExpressionReadMode::FallibleAliasInitializer)?
                    } else if raw_null {
                        self.point(
                            CfgPointKind::Evaluate(primary.clone()),
                            expression_span(primary),
                            scope,
                            BTreeSet::new(),
                        );
                        ReferenceValue::Other
                    } else if binding.fallible_assignment.is_some()
                        && matches!(primary, ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. })
                        && matches!(receiver.ty, ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::RuntimeArray { .. })
                    {
                        self.expression_with_read_mode(
                            primary,
                            scope,
                            primary_ordinal,
                            ExpressionReadMode::GuardedScalarAggregateInitializer,
                        )?
                    } else if ordinary_aggregate {
                        self.expression_with_read_mode(primary, scope, primary_ordinal, ExpressionReadMode::OrdinaryScalarAggregateInitializer)?
                    } else if walk_fallible_expression || runtime_returned || ordinary_runtime_receive {
                        let previous = self.capture_call_points;
                        self.capture_call_points = true;
                        let result = self.expression(primary, scope, primary_ordinal);
                        self.capture_call_points = previous;
                        result?
                    } else {
                        self.expression(primary, scope, primary_ordinal)?
                    };
                    if walk_fallible_expression {
                        let expression = self.guarded_numeric_expression(primary, primary_ordinal, &receiver.ty).filter(|_|
                            self.generic_parameters.is_empty() && binding_region.receivers().len() == 1
                            && outputs.secondary.is_empty());
                        match (binding.fallible_assignment, expression) {
                            (Some(_), Some(proof)) => self.aggregate_result.fallible_call_expressions.push(proof),
                            (None, Some(proof)) => self.aggregate_result.ordinary_fallible_expressions.push(vec![proof]),
                            (Some(_), None) => return Err(self.missing_context(expression_span(primary),
                                ReferenceAnalysisErrorReason::UnprovedFallibleCallInitializer)),
                            (None, None) => {}
                        }
                    }
                    let call_producer = if binding.output_origin
                        == ScalarBindingOutputOrigin::SingleExpression
                        && matches!(primary, ScalarExpression::Call { .. })
                    {
                        Some(self.cursor.expect("evaluated binding call"))
                    } else {
                        None
                    };
                    if let (None, ScalarExpression::Call { arguments, type_arguments, .. }, Some(point)) =
                        (binding.fallible_assignment, primary, call_producer)
                    {
                        if ordinary_root_binding
                            && type_arguments.is_empty() && !calls.declared_unsafe_calls.contains(&primary_ordinal.0)
                            && matches!(arguments.as_slice(), [argument]
                                if matches!(argument, ScalarExpression::Name { .. }) || aggregate::pure_widened_element(argument))
                            && calls.checked_call_signatures.get(&primary_ordinal).is_some_and(|signature|
                                signature.parameters.as_slice() == [ScalarType::U64]
                                    && matches!(signature.outputs.outputs.as_slice(), [output] if output.ty == receiver.ty))
                            && matches!(&self.cfg.points[point.0].kind, CfgPointKind::Call { output: 0,
                                target: ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. }, .. })
                        {
                            self.aggregate_result.ordinary_fallible_call_candidates.push(primary_ordinal);
                        }
                    }
                    if runtime_returned {
                        let paths = (!block.unsafe_context && self.generic_parameters.is_empty()
                            && binding_region.receivers().len() == 1 && outputs.secondary.is_empty())
                            .then(|| self.guarded_runtime_call_paths(primary, primary_ordinal, &receiver.ty)).flatten()
                            .ok_or_else(|| self.missing_context(expression_span(primary),
                                ReferenceAnalysisErrorReason::UnprovedFallibleCallInitializer))?;
                        if matches!(primary, ScalarExpression::If { .. }) {
                            self.aggregate_result.guarded_array_choices.insert(primary_ordinal.0);
                        }
                        self.aggregate_result.guarded_array_returns.extend(paths);
                    }
                    if let (Some(_), ScalarExpression::Call { arguments, .. }, false) =
                        (binding.fallible_assignment, primary, walk_fallible_expression || runtime_returned)
                    {
                        // Allocation-free callee evidence does not cover its
                        // caller-side argument evaluation.
                        let eligible = call_producer.filter(|point| self.generic_parameters.is_empty()
                            && arguments.iter().all(|argument| matches!(argument, ScalarExpression::Name { .. })
                                || aggregate::pure_widened_element(argument))
                            && is_integer_type(&receiver.ty) && binding_region.receivers().len() == 1
                            && outputs.secondary.is_empty()
                            && !calls.declared_unsafe_calls.contains(&primary_ordinal.0)
                            && calls.checked_call_signatures.get(&primary_ordinal).is_some_and(|signature|
                                signature.parameters.iter().all(aggregate::ordinary_value)
                                    && matches!(signature.outputs.outputs.as_slice(), [output] if output.ty == receiver.ty))
                            && matches!(&self.cfg.points[point.0].kind, CfgPointKind::Call { output: 0,
                                target: ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. }, .. }));
                        let Some(point) = eligible else {
                            return Err(self.missing_context(expression_span(primary),
                                ReferenceAnalysisErrorReason::UnprovedFallibleCallInitializer));
                        };
                        self.aggregate_result.fallible_call_initializers.push((point, primary_ordinal));
                    }
                    if let ([head, tail], Some(point)) = (binding_region.receivers(), call_producer) {
                        if aggregate::mixed_owner_result_lanes(&head.ty, &tail.ty).is_some()
                            && outputs.secondary.as_slice() == [BindingOutput::ProjectPrimary { output: 1 }]
                            && self.aggregate_facts.calls.contains_key(&point)
                            && matches!(&self.cfg.points[point.0].kind, CfgPointKind::Call {
                                target: ResolvedCallTarget::ModuleCallable {
                                    concrete: ConcreteCallSelection::Declaration, ..
                                }, ..
                            })
                            && calls.checked_call_signatures.get(&primary_ordinal).is_some_and(|signature|
                                matches!(signature.outputs.outputs.as_slice(), [first, second]
                                    if first.ty == head.ty && second.ty == tail.ty))
                        {
                            self.aggregate_result.mixed_result_bindings.insert(point);
                        }
                    }
                    if let Some(point) = call_producer {
                        let receivers = binding_region.receivers();
                        if let Some(&(ordinal, width)) = self.aggregate_facts.multi_owner_calls.get(&point) {
                            let signature = &calls.checked_call_signatures[&ordinal];
                            if !receivers.is_empty() && receivers.len() <= width
                                && outputs.secondary.len() + 1 == receivers.len()
                                && receivers.iter().zip(&signature.outputs.outputs).all(|(receiver, output)| receiver.ty == output.ty)
                                && outputs.secondary.iter().enumerate().all(|(index, output)|
                                    *output == (BindingOutput::ProjectPrimary { output: index + 1 }))
                            {
                                self.aggregate_result.multi_result_bindings.insert(point,
                                    receivers.iter().map(|receiver| ReferenceBindingId {
                                        source: self.source.clone(), function_span: self.function_span,
                                        declaration_span: receiver.span, block_span: block.span,
                                        kind: ReferenceBindingKind::Declared,
                                    }).collect());
                            }
                        }
                    }
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
                    let guarded_move = (|| {
                        if binding.is_allocation || binding.fallible_assignment.is_some() || block.unsafe_context
                            || !self.generic_parameters.is_empty() || binding_region.receivers().len() != 1
                            || !outputs.secondary.is_empty()
                        { return None; }
                        let ScalarExpression::Name { span, .. } = primary else { return None; };
                        let Some(PlainNameIdentity::Lexical(source)) = self.prepared_name_identity(primary_ordinal, *span).ok().flatten()
                        else { return None; };
                        let site = self.proof_points.fallible_allocation_bindings.as_ref()?.get(source)?;
                        aggregate::same_runtime_array_type(&site.ty, &receiver.ty).then(|| source.clone())
                    })();
                    let ordinary_array_receive = ordinary_runtime_receive
                        .then(|| self.guarded_runtime_call_paths(primary, primary_ordinal, &receiver.ty))
                        .flatten();
                    let widened_move = outputs.secondary.is_empty()
                        && self.exact_widened_binding_move(binding, primary, primary_ordinal, block.span);
                    self.move_value_with_widened_receipt(primary, scope, primary_ordinal, widened_move)?;
                    let id = ReferenceBindingId {
                        source: self.source.clone(),
                        function_span: self.function_span,
                        declaration_span: receiver.span,
                        block_span: block.span,
                        kind: ReferenceBindingKind::Declared,
                    };
                    self.binding_types.record(id.clone(), receiver.ty.clone(), self.deferred_local_reborrow.then_some(scope));
                    if let Some(paths) = ordinary_array_receive {
                        self.proof_points.ordinary_array_receivers.insert(id.clone(), primary_ordinal);
                        self.aggregate_result.ordinary_array_receivers.push((
                            primary_ordinal, aggregate::OrdinaryArrayDestination::Declaration(receiver.span), paths,
                        ));
                    }
                    if let Some(source) = guarded_move {
                        declares_guarded_owner = true;
                        let read = initializer.ok_or_else(|| self.missing_context(expression_span(primary),
                            ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer))?;
                        let moved = self.cursor.filter(|point| *point != read)
                            .ok_or_else(|| self.missing_context(expression_span(primary),
                                ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer))?;
                        self.cfg.allocations.insert(id.clone());
                        self.proof_points.guarded_runtime_receivers.insert(id.clone());
                        self.proof_points.guarded_runtime_moves.insert(read, GuardedRuntimeMove {
                            source, target: id.clone(), target_ty: receiver.ty.clone(), rhs: primary_ordinal.0, replacement: None, moved, scope,
                        });
                    }
                    if let Some(owner) = alias_owner {
                        self.proof_points.fallible_aliases.insert(id.clone(), owner);
                    }
                    if let Some(owner) = widened_receiver_source {
                        let point = call_producer.filter(|point| self.aggregate_facts.array_calls.get(point)
                            .is_some_and(|(actual, _, source)| *actual == primary_ordinal && source.declaration_span == owner))
                            .ok_or_else(|| self.missing_context(expression_span(primary),
                                ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer))?;
                        self.aggregate_result.widened_owning_receives.push((point, primary_ordinal, owner, receiver.span, None));
                    }
                    let widened_fixed_copy = if let Some((_, count)) = calls.structure.checked_types.expressions.get(&primary_ordinal)
                        .and_then(|actual| super::widened_fixed_copy(binding, actual))
                    {
                        let ScalarExpression::Name { span, .. } = primary else { unreachable!() };
                        let exact = self.prepared_name_identity(primary_ordinal, *span)?;
                        let Some(PlainNameIdentity::Lexical(source)) = exact.filter(|identity|
                            self.generic_parameters.is_empty()
                                && matches!(identity, PlainNameIdentity::Lexical(source)
                                    if self.binding_types.get(source).and_then(|ty| super::widened_fixed_copy(binding, ty))
                                        .is_some_and(|(_, actual)| actual == count)))
                        else {
                            return Err(self.missing_context(*span, ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer));
                        };
                        self.aggregate_result.widened_fixed_copies.insert((receiver.span.start, receiver.span.end),
                            (source.declaration_span, count));
                        true
                    } else { false };
                    if let Some((owner, count)) = fixed_copy_receiver {
                        let point = call_producer.ok_or_else(|| self.missing_context(expression_span(primary),
                            ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer))?;
                        let CfgPointKind::Call { argument_points, .. } = &self.cfg.points[point.0].kind
                        else { return Err(self.missing_context(expression_span(primary), ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer)); };
                        if !matches!(argument_points.as_slice(), [argument]
                            if matches!(&self.cfg.points[argument.0].kind, CfgPointKind::Read { binding } if binding.declaration_span == owner))
                        { return Err(self.missing_context(expression_span(primary), ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer)); }
                        self.aggregate_result.fixed_copy_owning_arguments.insert(primary_ordinal.0, count);
                        self.aggregate_result.widened_owning_receives.push((point, primary_ordinal, owner, receiver.span, None));
                    }
                    let widened = widened_join || widened_move || widened_receiver_source.is_some()
                        || fixed_copy_receiver.is_some() || widened_fixed_copy
                        || super::widened_local_literal(binding).is_some();
                    if widened {
                        self.widened_arrays.get_or_insert_with(HashSet::new).insert(id.clone());
                    }
                    if binding.is_allocation || widened {
                        self.cfg.allocations.insert(id.clone());
                    }
                    let runtime_literal_length = super::guarded_runtime_literal(binding).and_then(|(_, values, _)| {
                        let (_, length) = super::fixed_copy_to_runtime(&receiver.ty,
                            calls.structure.checked_types.expressions.get(&primary_ordinal)?)?;
                        (length == values.len() as u64).then_some(length)
                    });
                    if binding.fallible_assignment.is_some()
                        && (runtime_returned || runtime_literal_length.is_some() || is_integer_type(&receiver.ty)
                            || matches!(receiver.ty, ScalarType::Struct(_) | ScalarType::Array { .. } | ScalarType::Bool))
                    {
                        declares_guarded_owner = true;
                        let whole_copy = matches!(&receiver.ty, ScalarType::Struct(_))
                            && matches!(copy_policy(&receiver.ty, self.structs, self.enums, &self.generic_parameters),
                                Ok(CopyPolicy::Copy));
                        self.proof_points.fallible_allocation_bindings.get_or_insert_with(HashMap::new).insert(
                            id.clone(), super::control::GuardedAllocationBinding {
                                scope, ty: receiver.ty.clone(), whole_copy, runtime_literal_length, runtime_returned,
                            },
                        );
                        self.local_borrow = None;
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
                    let widened_factory = (|| {
                        if scope != ReferenceScopeId(0) || !self.generic_parameters.is_empty()
                            || item_index != 0 || !calls.boundary.parameters.is_empty()
                            || !outputs.secondary.is_empty() || block.unsafe_context
                            || !matches!(&calls.named_place_roots, NamedPlaceRootFacts::Prepared(_))
                        { return None; }
                        let [RootOutputType { output: 0, ty }] = calls.boundary.root_outputs.as_slice()
                        else { return None };
                        let [_, BlockItem { kind: BlockItemKind::FinalOutput(output), .. }] =
                            region.items.as_slice() else { return None };
                        let [_, ScalarBlockItem::Expression(ScalarExpression::Name { name, .. })] =
                            block.items.as_slice() else { return None };
                        if output.output != 0 || *name != receiver.name
                            || !aggregate::same_runtime_array_type(&receiver.ty, ty)
                        { return None; }
                        Some((output.expression, Self::pure_widened_literal_count(binding)?, initializer?))
                    })();
                    self.record_binding_definition(position, 0, binding.is_allocation || widened_factory.is_some(), initializer);
                    if let Some((output, count, placeholder)) = widened_factory {
                        let bind = self.cursor.expect("widened array definition");
                        self.aggregate_facts.literal_array_bind = Some(aggregate::LiteralArrayBind {
                            bind, placeholder, position, length_occurrence: primary_ordinal,
                            extent: aggregate::FreshArrayExtent::Literal(count),
                        });
                        self.aggregate_facts.widened_literal_factory = true;
                        self.aggregate_result.widened_literal_factory = Some((receiver.span, output));
                    }
                    // The existing birth observer is selected while still at
                    // the allocation, before the write or final result is visited.
                    // Use the prepared final-output role to opt in only the new
                    // exact three-item scalar observation corridor.
                    let returned_extent_tail = scope == ReferenceScopeId(0)
                        && matches!(calls.boundary.root_outputs.as_slice(),
                            [RootOutputType { output: 0, ty: ScalarType::U64 }])
                        && matches!((region.items.as_slice(), block.items.as_slice()),
                            ([_, BlockItem { kind: BlockItemKind::Assignment { .. }, .. },
                                BlockItem { kind: BlockItemKind::FinalOutput(slot), .. }],
                             [_, _, ScalarBlockItem::Expression(expression)])
                                if slot.output == 0
                                    && matches!(expression, ScalarExpression::Call { .. })
                                    && matches!(self.call_target(expression, slot.expression),
                                        Ok(ResolvedCallTarget::Core(CoreOperationId::ArrayLen))));
                    let returned_index_tail = scope == ReferenceScopeId(0)
                        && matches!(calls.boundary.root_outputs.as_slice(), [output] if aggregate::ordinary_value(&output.ty))
                        && matches!((region.items.as_slice(), block.items.as_slice()),
                            ([_, BlockItem { kind: BlockItemKind::Assignment { .. }, .. },
                                BlockItem { kind: BlockItemKind::FinalOutput(slot), .. }],
                             [_, _, ScalarBlockItem::Expression(ScalarExpression::IndexedRead { place, .. })])
                                if slot.output == 0 && matches!(place, ScalarPlace::Index { base, index, .. }
                                    if matches!(base.as_ref(), ScalarPlace::Name { .. })
                                        && Self::returned_array_index_selector_syntax(index)));
                    if binding.is_allocation
                        && (returned_extent_tail || returned_index_tail || (region.items.len() == 2
                            && matches!(calls.boundary.root_outputs.as_slice(),
                                [] | [RootOutputType { output: 0, ty: ScalarType::RuntimeArray { .. } }])))
                    {
                        self.observe_literal_array_bind(
                            position, scope, binding, &binding_region.initialization, initializer,
                        );
                    }
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
                    let mode = if projected_origins && matches!(kind, BlockItemKind::FinalOutput(_)) {
                        ExpressionReadMode::ProjectedOrigins
                    } else {
                        ExpressionReadMode::Ordinary
                    };
                    let guarded_owner_choice = !block.unsafe_context && self.generic_parameters.is_empty()
                        && scope == ReferenceScopeId(0)
                        && matches!(region.items.as_slice(), [BlockItem { kind: BlockItemKind::FinalOutput(row), .. }]
                            if row.output == 0 && row.expression == ordinal)
                        && matches!(calls.boundary.parameters.as_slice(), [formal] if formal.ty == ScalarType::U64)
                        && matches!(expression, ScalarExpression::If { .. })
                        && matches!(calls.boundary.root_outputs.as_slice(), [output]
                            if matches!(&output.ty, ScalarType::RuntimeArray { element, .. } if aggregate::ordinary_value(element)))
                        && super::typecheck::guarded_runtime_rhs(expression);
                    let scalar_tail = self.capture_private_scalar_tail && !block.unsafe_context
                        && scope == ReferenceScopeId(0)
                        && matches!(calls.boundary.parameters.as_slice(), [formal] if formal.ty == ScalarType::U64)
                        && matches!(region.items.as_slice(), [BlockItem { kind: BlockItemKind::FinalOutput(row), .. }]
                            if row.output == 0 && row.expression == ordinal)
                        && matches!(expression, ScalarExpression::Binary { .. } | ScalarExpression::If { .. })
                        && matches!(calls.boundary.root_outputs.as_slice(),
                            [output] if matches!(output.ty, ScalarType::U64 | ScalarType::I32 | ScalarType::Bool));
                    let previous = self.capture_call_points;
                    self.capture_call_points |= guarded_owner_choice;
                    let tail_active = self.private_scalar_tail_active;
                    self.private_scalar_tail_active |= scalar_tail;
                    let value = self.expression_with_read_mode(expression, scope, ordinal, mode);
                    self.capture_call_points = previous;
                    self.private_scalar_tail_active = tail_active;
                    let value = value?;
                    if guarded_owner_choice {
                        let ty = &calls.boundary.root_outputs[0].ty;
                        if let Some(paths) = self.guarded_runtime_call_paths(expression, ordinal, ty) {
                            self.aggregate_result.guarded_array_producer_choice = Some((ordinal, paths));
                        }
                    }
                    let value_point = self.cursor.expect("evaluated output expression");
                    let widened_arm = self.aggregate_result.widened_join_arms.get(&ordinal.0)
                        .is_some_and(|declaration| matches!(expression, ScalarExpression::Name { span, .. }
                            if matches!(self.prepared_name_identity(ordinal, *span),
                                Ok(Some(PlainNameIdentity::Lexical(source)))
                                    if source.block_span == block.span
                                        && source.declaration_span == *declaration
                                        && self.widened_arrays.as_ref().is_some_and(|owners| owners.contains(source)))));
                    let widened_factory = self.aggregate_result.widened_literal_factory
                        .is_some_and(|(declaration, output)| output == ordinal
                            && matches!(expression, ScalarExpression::Name { span, .. }
                                if matches!(self.prepared_name_identity(ordinal, *span),
                                    Ok(Some(PlainNameIdentity::Lexical(source)))
                                        if source.block_span == block.span
                                            && source.declaration_span == declaration
                                            && self.widened_arrays.as_ref().is_some_and(|owners| owners.contains(source)))));
                    self.move_value_with_widened_receipt(expression, scope, ordinal, widened_arm || widened_factory)?;
                    if let BlockItemKind::FinalOutput(slot) = kind {
                        if scope == ReferenceScopeId(0) && slot.output == 0 && region.items.len() == 1
                            && matches!(calls.boundary.root_outputs.as_slice(), [output] if output.ty == ScalarType::U64)
                            && matches!(calls.boundary.parameters.as_slice(), [formal]
                                if matches!(&formal.ty, ScalarType::CheckedReference {
                                    mutability: ScalarReferenceMutability::Shared, inner,
                                } if matches!(inner.as_ref(), ScalarType::RuntimeArray { .. })))
                            && !calls.declared_unsafe_calls.contains(&slot.expression.0)
                            && matches!(&self.cfg.points[value_point.0].kind, CfgPointKind::Call { output: 0,
                                target: ResolvedCallTarget::ModuleCallable { concrete: ConcreteCallSelection::Declaration, .. }, .. })
                        {
                            self.aggregate_facts.borrowed_extent_forward = Some((value_point, slot.expression));
                        }
                        if scope == ReferenceScopeId(0) && slot.output == 0
                            && matches!(calls.boundary.root_outputs.as_slice(), [output] if output.ty == ScalarType::U64)
                            && !calls.declared_unsafe_calls.contains(&slot.expression.0)
                            && calls.checked_call_signatures.get(&slot.expression).is_some_and(|signature|
                                matches!(signature.parameters.as_slice(), [ScalarType::CheckedReference {
                                    mutability: ScalarReferenceMutability::Shared, inner,
                                }] if matches!(inner.as_ref(), ScalarType::RuntimeArray { .. })))
                        {
                            if let [
                                BlockItem { kind: BlockItemKind::Binding(root), .. },
                                aliases @ ..,
                                BlockItem { kind: BlockItemKind::FinalOutput(_), .. },
                            ] = region.items.as_slice() {
                                if matches!(root.initialization, BindingInitialization::Allocation { .. })
                                    && aliases.iter().all(|item| matches!(&item.kind,
                                        BlockItemKind::Binding(binding) if binding.receivers().len() == 1
                                            && matches!(&binding.initialization, BindingInitialization::Outputs(init)
                                                if init.secondary.is_empty())))
                                {
                                    if let CfgPointKind::Call { output: 0, target: ResolvedCallTarget::ModuleCallable {
                                        concrete: ConcreteCallSelection::Declaration, ..
                                    }, argument_points, .. } = &self.cfg.points[value_point.0].kind {
                                        if let [argument] = argument_points.as_slice() {
                                            if let Some((owner, alias)) = self.borrowed_extent_argument(*argument, aliases.len()) {
                                                self.aggregate_facts.borrowed_extent_call = Some((value_point, slot.expression, owner, alias));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if scope == ReferenceScopeId(0) && slot.output == 0
                            && self.aggregate_facts.literal_array_unit.is_none()
                            && matches!(calls.boundary.root_outputs.as_slice(), [output] if output.ty == ScalarType::U64)
                            && matches!(&self.cfg.points[value_point.0].kind, CfgPointKind::Call {
                                target: ResolvedCallTarget::Core(CoreOperationId::ArrayLen), argument_points, ..
                            } if matches!(argument_points.as_slice(), [borrow] if
                                matches!(&self.cfg.points[borrow.0].kind, CfgPointKind::Borrow { place, .. }
                                    if place.projections.is_empty() && (self.aggregate_result.widened_owning_roundtrip.is_some_and(|(_, _, owner)|
                                        owner == place.binding.declaration_span)
                                    || self.aggregate_facts.allocations.get(&place.binding)
                                        .is_some_and(|bind| {
                                            (self.aggregate_facts.literal_array_write.is_some()
                                                && self.aggregate_facts.literal_array_bind.is_some_and(|allocation| allocation.bind == *bind))
                                            || (matches!(region.items.as_slice(),
                                                [BlockItem { kind: BlockItemKind::Binding(_), .. },
                                                 BlockItem { kind: BlockItemKind::Assignment { .. }, .. },
                                                 BlockItem { kind: BlockItemKind::FinalOutput(_), .. }])
                                                && self.aggregate_facts.array_installs.len() == 1
                                                && self.aggregate_facts.array_installs.first_key_value().is_some_and(|(install, _)|
                                                    matches!(&self.cfg.points[install.0].kind, CfgPointKind::Assign { target, .. }
                                                        if target.binding == place.binding)))
                                        })))))
                        {
                            // Only pending authority: exact receipt replay and the
                            // all-use transported ABI must close this occurrence
                            // before any LLVM declaration can be emitted.
                            self.aggregate_facts.literal_array_extent_output = Some((value_point, slot.expression));
                            self.proof_points.pending_returned_extent = Some(value_point);
                        }
                        if scope == ReferenceScopeId(0) && slot.output == 0
                            && self.aggregate_facts.literal_array_unit.is_none()
                            && self.aggregate_facts.literal_array_write.is_some()
                            && matches!(calls.boundary.root_outputs.as_slice(), [output] if aggregate::ordinary_value(&output.ty))
                            && matches!(expression, ScalarExpression::IndexedRead { place: ScalarPlace::Index { base, index, .. }, .. }
                                if matches!(base.as_ref(), ScalarPlace::Name { .. }) && Self::returned_array_index_selector_syntax(index))
                            && self.names_literal_array_owner_at_index(value_point)
                        {
                            let pending = self.returned_array_index_observation(expression, slot.expression, value_point)
                                .ok_or_else(|| self.missing_context(expression_span(expression),
                                    ReferenceAnalysisErrorReason::UnprovedReturnedArraySelector))?;
                            self.aggregate_facts.literal_array_index_output = Some(pending);
                        }
                        if scope == ReferenceScopeId(0) && slot.output == 0
                            && self.aggregate_facts.literal_array_bind.is_some()
                            && matches!(&self.cfg.points[value_point.0].kind, CfgPointKind::Read { binding }
                                if self.cursor.is_some_and(|moved| moved != value_point
                                    && matches!(&self.cfg.points[moved.0].kind,
                                        CfgPointKind::Move { place, owner, ty: ScalarType::RuntimeArray { .. } }
                                            if owner == binding && place.binding == *binding && place.projections.is_empty())))
                        {
                            self.aggregate_facts.literal_array_value = Some((value_point, slot.expression));
                        }
                        if scope == ReferenceScopeId(0) && slot.output == 0
                            && self.aggregate_facts.array_formal.is_some()
                        {
                            self.aggregate_facts.array_return = self.cursor
                                .filter(|moved| *moved != value_point)
                                .map(|moved| (value_point, moved));
                        }
                        let ty = if scope == ReferenceScopeId(0) {
                            &calls.boundary.root_outputs[slot.output].ty
                        } else {
                            &block.final_output_values[slot.output].ty
                        };
                        // A nested block's stored output annotation can be Error
                        // even when the occurrence has a resolved nominal type.
                        if scope != ReferenceScopeId(0) && slot.output == 0
                            && matches!(calls.structure.checked_types.expressions.get(&slot.expression),
                                Some(ScalarType::Struct(_)))
                            && matches!(expression, ScalarExpression::Name { .. })
                        {
                            self.aggregate_facts.structured_local_candidates.insert(value_point, slot.expression);
                        }
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
                    let guarded_replacement = (|| {
                        if block.unsafe_context || !self.generic_parameters.is_empty() { return None; }
                        let ([target], [ScalarExpression::Name { span, .. }]) =
                            (assignment.targets.as_slice(), assignment.values.as_slice()) else { return None; };
                        if !matches!(target.place, ScalarPlace::Name { .. }) { return None; }
                        let rhs = group.rhs_evaluations().next()?.expression;
                        let destination = group.destinations().next()?.destination;
                        let PlainNameIdentity::Lexical(source) = self.prepared_name_identity(rhs, *span).ok().flatten()?
                        else { return None; };
                        let DirectDestinationIdentity::Existing(target) = calls.prepared_direct_destination(destination)?
                        else { return None; };
                        let source_site = self.proof_points.fallible_allocation_bindings.as_ref()?.get(source)?;
                        let ty = self.binding_types.get(target)?;
                        (target.block_span == block.span
                            && self.proof_points.guarded_runtime_receivers.contains(target)
                            && aggregate::same_runtime_array_type(&source_site.ty, ty))
                            .then(|| (source.clone(), target.clone(), ty.clone(), rhs, destination))
                    })();
                    let widened_rebind = (|| {
                        let owners = self.widened_arrays.as_ref()?;
                        if scope != ReferenceScopeId(0) || !self.generic_parameters.is_empty() { return None; }
                        let ([target], [ScalarExpression::Name { span, .. }]) =
                            (assignment.targets.as_slice(), assignment.values.as_slice())
                        else { return None };
                        if !matches!(target.place, ScalarPlace::Name { .. }) { return None; }
                        let evaluation = group.rhs_evaluations().next()?;
                        let destination = group.destinations().next()?.destination;
                        let Some(PlainNameIdentity::Lexical(source)) =
                            self.prepared_name_identity(evaluation.expression, *span).ok().flatten()
                        else { return None };
                        let Some(DirectDestinationIdentity::Existing(target)) = calls.prepared_direct_destination(destination)
                        else { return None };
                        (source != target && source.block_span == block.span && target.block_span == block.span
                            && owners.contains(source) && owners.contains(target)
                            && self.binding_types.get(source).zip(self.binding_types.get(target))
                                .is_some_and(|(source, target)| aggregate::same_runtime_array_type(source, target)))
                            .then_some((destination, source.declaration_span, target.declaration_span))
                    })();
                    let widened_roundtrip = (|| {
                        if scope != ReferenceScopeId(0) || !self.generic_parameters.is_empty()
                            || item_index != 1 || region.items.len() != 3
                            || !matches!(calls.boundary.root_outputs.as_slice(), [output] if output.ty == ScalarType::U64)
                        { return None; }
                        let [_, _, BlockItem { kind: BlockItemKind::FinalOutput(observe), .. }] =
                            region.items.as_slice() else { return None };
                        let [_, _, ScalarBlockItem::Expression(observation)] = block.items.as_slice() else { return None };
                        if !matches!(self.call_target(observation, observe.expression),
                            Ok(ResolvedCallTarget::Core(CoreOperationId::ArrayLen))) { return None; }
                        let ([target], [value]) = (assignment.targets.as_slice(), assignment.values.as_slice())
                        else { return None; };
                        if !matches!(target.place, ScalarPlace::Name { .. }) { return None; }
                        let occurrence = group.rhs_evaluations().next()?.expression;
                        let source = self.widened_owning_argument(value, occurrence, block.span)?;
                        let destination = group.destinations().next()?.destination;
                        let DirectDestinationIdentity::Existing(binding) = calls.prepared_direct_destination(destination)?
                        else { return None; };
                        (source == binding).then_some((occurrence, destination, binding.declaration_span))
                    })();
                    let widened_owning_replace = (|| {
                        if scope != ReferenceScopeId(0) { return None; }
                        let owners = self.widened_arrays.as_ref()?;
                        let ([target], [value]) = (assignment.targets.as_slice(), assignment.values.as_slice())
                        else { return None; };
                        if !matches!(target.place, ScalarPlace::Name { .. }) { return None; }
                        let call = group.rhs_evaluations().next()?.expression;
                        let source = self.widened_owning_argument(value, call, block.span)?;
                        let destination = group.destinations().next()?.destination;
                        let DirectDestinationIdentity::Existing(target) = calls.prepared_direct_destination(destination)?
                        else { return None; };
                        (source != target && owners.contains(target) && target.block_span == block.span
                            && self.binding_types.get(source).zip(self.binding_types.get(target))
                                .is_some_and(|(a, b)| aggregate::same_runtime_array_type(a, b)))
                            .then_some((call, destination, source.declaration_span, target.declaration_span))
                    })();
                    let widened_join_replace = (|| {
                        let owners = self.widened_arrays.as_ref()?;
                        let ([target], [value @ ScalarExpression::If { .. }]) =
                            (assignment.targets.as_slice(), assignment.values.as_slice()) else { return None; };
                        if !matches!(target.place, ScalarPlace::Name { .. }) { return None; }
                        let evaluation = group.rhs_evaluations().next()?.expression;
                        let destination = group.destinations().next()?.destination;
                        let DirectDestinationIdentity::Existing(target) = calls.prepared_direct_destination(destination)?
                        else { return None; };
                        if target.block_span != block.span || !owners.contains(target) { return None; }
                        let ty = self.binding_types.get(target)?;
                        let arms = self.widened_join_inputs(value, ty, evaluation, scope)?;
                        Some((destination, evaluation, target.declaration_span, arms))
                    })();
                    if let Some((_, _, _, arms)) = widened_join_replace {
                        self.aggregate_result.widened_join_arms.extend(arms);
                    }
                    let descriptor_replace = (|| {
                        if block.unsafe_context || !self.generic_parameters.is_empty()
                            || group.projected_outputs().len() != 0 { return None; }
                        let ([target], [value]) = (assignment.targets.as_slice(), assignment.values.as_slice())
                        else { return None; };
                        if !matches!(target.place, ScalarPlace::Name { .. })
                            || !super::typecheck::guarded_runtime_rhs(value) { return None; }
                        let rhs = group.rhs_evaluations().next()?.expression;
                        let destination = group.destinations().next()?.destination;
                        let DirectDestinationIdentity::Existing(owner) = calls.prepared_direct_destination(destination)?
                        else { return None; };
                        if owner.block_span != block.span
                            || !self.widened_arrays.as_ref()?.contains(owner)
                        { return None; }
                        let ty = self.binding_types.get(owner)?;
                        matches!(ty, ScalarType::RuntimeArray { element, .. } if aggregate::ordinary_value(element))
                            .then(|| (destination, rhs, owner.declaration_span, ty.clone()))
                    })();
                    let mut descriptor_paths = None;
                    let ordinary_aggregate_replacement = (|| {
                        if block.unsafe_context || !self.generic_parameters.is_empty()
                            || group.projected_outputs().len() != 0
                        { return None; }
                        let ([target], [value]) =
                            (assignment.targets.as_slice(), assignment.values.as_slice()) else { return None; };
                        if !matches!(target.place, ScalarPlace::Name { .. })
                            || !matches!(value, ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. })
                        { return None; }
                        let evaluation = group.rhs_evaluations().next()?;
                        let destination = group.destinations().next()?.destination;
                        let DirectDestinationIdentity::Existing(binding) = calls.prepared_direct_destination(destination)?
                        else { return None; };
                        let ty = self.binding_types.get(binding)?;
                        (matches!((ty, value),
                            (ScalarType::Struct(_), ScalarExpression::StructLiteral { .. })
                            | (ScalarType::Array { .. }, ScalarExpression::ArrayLiteral { .. }))
                            && calls.structure.checked_types.expressions.get(&evaluation.expression) == Some(ty))
                            .then_some(evaluation.expression)
                    })();
                    let scalar_rhs_eligible = !block.unsafe_context && self.generic_parameters.is_empty()
                        && group.projected_outputs().len() == 0;
                    let mut scalar_rhs = Some(Vec::new());
                    let mut values = Vec::new();
                    for evaluation in group.rhs_evaluations() {
                        let value = &assignment.values[evaluation.position];
                        let ordinary_payload = if scalar_rhs_eligible && scalar_rhs.is_some()
                            && matches!(value, ScalarExpression::Call { .. } | ScalarExpression::If { .. } | ScalarExpression::Binary { .. })
                        {
                            group.destinations().as_slice().get(evaluation.position).and_then(|slot| {
                                let AssignmentReplacement::Resolved(ty @ (ScalarType::U64 | ScalarType::I32 | ScalarType::Bool)) = &slot.replacement
                                else { return None; };
                                let target = &assignment.targets[slot.position].place;
                                let exact = match target {
                                    ScalarPlace::Name { .. } => matches!(calls.prepared_direct_destination(slot.destination),
                                        Some(DirectDestinationIdentity::Existing(_))),
                                    ScalarPlace::Index { .. } => {
                                        let root = self.place_root(target, slot.destination);
                                        matches!(root.place, ScalarPlace::Name { span, .. }
                                            if matches!(self.prepared_place_root(root.ordinal, *span),
                                                Ok(Some(PreparedPlaceRoot::Lexical(_)))))
                                    }
                                    _ => false,
                                };
                                (exact && calls.structure.checked_types.expressions.get(&evaluation.expression) == Some(ty)).then_some(ty)
                            })
                        } else { None };
                        let exact_move = widened_roundtrip.map(|(call, _, owner)| (call, owner))
                            .or(widened_owning_replace.map(|(call, _, owner, _)| (call, owner)));
                        let mode = match exact_move {
                            Some((call, owner)) if call == evaluation.expression => ExpressionReadMode::WidenedOwningRoundtrip(owner),
                            _ if ordinary_aggregate_replacement == Some(evaluation.expression) =>
                                ExpressionReadMode::OrdinaryScalarAggregateInitializer,
                            _ => ExpressionReadMode::Ordinary,
                        };
                        let capture = self.capture_call_points;
                        let descriptor_type = descriptor_replace.as_ref().filter(|(_, rhs, _, _)| *rhs == evaluation.expression)
                            .map(|(_, _, _, ty)| ty);
                        self.capture_call_points |= ordinary_payload.is_some() || descriptor_type.is_some();
                        let result = self.expression_with_read_mode(value, scope, evaluation.expression, mode);
                        self.capture_call_points = capture;
                        let result = result?;
                        if let Some(ty) = descriptor_type {
                            descriptor_paths = self.guarded_runtime_call_paths(value, evaluation.expression, ty);
                        }
                        if let Some(ty) = ordinary_payload {
                            if let Some(proof) = self.guarded_numeric_expression(value, evaluation.expression, ty) {
                                scalar_rhs.as_mut().expect("eligible ordinary RHS group").push(proof);
                            } else {
                                // Do not partially enroll a group whose other
                                // candidate requires an unsupported expression.
                                scalar_rhs = None;
                            }
                        }
                        let rhs_point = self.cursor.expect("evaluated assignment output");
                        self.move_value_with_widened_receipt(value, scope, evaluation.expression, widened_rebind.is_some())?;
                        if let Some((source, target, ty, rhs, destination)) = &guarded_replacement {
                            let moved = self.cursor.filter(|point| *point != rhs_point)
                                .ok_or_else(|| self.missing_context(expression_span(value),
                                    ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer))?;
                            self.proof_points.guarded_runtime_moves.insert(rhs_point, GuardedRuntimeMove {
                                source: source.clone(), target: target.clone(), target_ty: ty.clone(), rhs: rhs.0,
                                replacement: Some(destination.0), moved, scope,
                            });
                        }
                        values.push((result, rhs_point));
                    }
                    if let Some(proofs) = scalar_rhs.filter(|proofs| !proofs.is_empty()) {
                        self.aggregate_result.ordinary_fallible_expressions.push(proofs);
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
                        self.proof_points.has_grouped_assignment_capture = Some(true);
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
                    let primary_array_producer = group
                        .rhs_evaluations()
                        .next()
                        .filter(|evaluation| assignment.values.len() == 1 && evaluation.position == 0)
                        .map(|evaluation| evaluation.expression);
                    let mut simple_destinations = primary_array_producer.is_some();
                    for ((slot, (value, rhs_point)), destination) in
                        group.installs().zip(values).zip(destinations)
                    {
                        let existing_prepared = matches!(&destination, ReferenceAssignmentDestination::PreparedName(DirectDestinationIdentity::Existing(_)));
                        simple_destinations &= existing_prepared;
                        let prepared_local_introduction = matches!(&destination,
                            ReferenceAssignmentDestination::PreparedName(DirectDestinationIdentity::Introduce { .. }));
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
                        let emitted = if let ScalarPlace::Name { name, span } = &target.place {
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
                                let install = self.point(
                                    CfgPointKind::Evaluate(ScalarExpression::Name {
                                        name: name.clone(),
                                        span: *span,
                                    }),
                                    *span,
                                    scope,
                                    BTreeSet::new(),
                                );
                                self.record_frozen_install(slot.destination, FrozenInstall { rhs: rhs_point, install });
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
                                self.binding_types.record(id.clone(), ty.clone(), (prepared_local_introduction && self.deferred_local_reborrow).then_some(scope));
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
                            let widened_rebind = widened_rebind.filter(|(destination, _, target)|
                                *destination == slot.destination && *target == id.declaration_span);
                            let widened_roundtrip = widened_roundtrip.filter(|(_, destination, owner)|
                                *destination == slot.destination && *owner == id.declaration_span);
                            let widened_owning_replace = widened_owning_replace.filter(|(_, destination, _, target)|
                                *destination == slot.destination && *target == id.declaration_span);
                            let widened_join_replace = widened_join_replace.filter(|(destination, _, target, _)|
                                *destination == slot.destination && *target == id.declaration_span && capture.is_none());
                            let descriptor_replace = descriptor_replace.as_ref()
                                .filter(|(destination, _, target, _)| *destination == slot.destination
                                    && *target == id.declaration_span && capture.is_none())
                                .zip(descriptor_paths.take());
                            if widened_rebind.is_none() && widened_roundtrip.is_none() && widened_owning_replace.is_none()
                                && widened_join_replace.is_none() && descriptor_replace.is_none()
                                && self.widened_arrays.as_ref().is_some_and(|owners| owners.contains(&id))
                                && matches!(&assign, CfgPointKind::Assign { target, .. } if target.projections.is_empty())
                            {
                                return Err(self.missing_context(*span, ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer));
                            }
                            let guarded_install = guarded_replacement.as_ref()
                                .is_some_and(|(_, owner, _, _, destination)| owner == &id && *destination == slot.destination && capture.is_none());
                            if !guarded_install && self.proof_points.guarded_runtime_receivers.contains(&id)
                                && matches!(&assign, CfgPointKind::Assign { target, .. } if target.projections.is_empty())
                            {
                                return Err(self.missing_origin(*span, id, ReferenceAnalysisErrorReason::UnsupportedFallibleAllocationAccess));
                            }
                            let emitted = self.emit_direct_assignment(id, assign, rhs_point, value, *span, scope)?;
                            if guarded_install {
                                self.proof_points.widened_rebind_points.get_or_insert_with(HashSet::new).insert(emitted.install);
                            }
                            if let Some((destination, source, target)) = widened_rebind {
                                self.proof_points.widened_rebind_points.get_or_insert_with(HashSet::new).insert(emitted.install);
                                self.aggregate_result.widened_rebinds.insert(destination.0,
                                    aggregate::ClosedWidenedRebind { point: emitted.install, source, target });
                            }
                            if let Some((call, _, owner)) = widened_roundtrip {
                                self.aggregate_result.widened_owning_roundtrip = Some((call, emitted.install, owner));
                            }
                            if let Some((call, destination, source, target)) = widened_owning_replace {
                                if !self.aggregate_facts.array_calls.get(&emitted.rhs)
                                    .is_some_and(|(actual, _, binding)| *actual == call && binding.declaration_span == source)
                                {
                                    return Err(self.missing_context(*span, ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer));
                                }
                                self.proof_points.widened_rebind_points.get_or_insert_with(HashSet::new).insert(emitted.install);
                                self.aggregate_result.widened_owning_receives.push((emitted.rhs, call, source, target,
                                    Some((emitted.install, destination.0))));
                            }
                            if let Some((destination, rhs, target, _)) = widened_join_replace {
                                self.proof_points.widened_rebind_points.get_or_insert_with(HashSet::new).insert(emitted.install);
                                self.aggregate_result.widened_join_replacements.insert(destination.0, (rhs.0, emitted.install, target));
                            }
                            if let Some(((destination, rhs, declaration, _), paths)) = descriptor_replace {
                                self.proof_points.widened_rebind_points.get_or_insert_with(HashSet::new).insert(emitted.install);
                                self.aggregate_result.ordinary_array_receivers.push((*rhs,
                                    aggregate::OrdinaryArrayDestination::Replace {
                                        destination: *destination, install: emitted.install, declaration: *declaration,
                                    }, paths));
                            }
                            self.author_direct_assignment(emitted, existing_prepared && capture.is_none()
                                && assignment.targets.len() == 1 && assignment.values.len() == 1);
                            emitted
                        } else {
                            if let ReferenceAssignmentDestination::Through(pointer_point) =
                                destination
                            {
                                let assign = CfgPointKind::AssignThrough {
                                    place: self.proof_place(&target.place, slot.destination).into_owned(),
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
                                if capture.is_none()
                                    && (self.proof_points.pending_named_reborrow.is_some() || self.has_projected_child || self.has_joined_root_reborrow)
                                    && matches!(&self.cfg.points[pointer_point.0].kind,
                                        CfgPointKind::Read { binding } if matches!(self.binding_types.get(binding),
                                            Some(ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner })
                                                if match &target.place {
                                                    ScalarPlace::Dereference { pointer, .. } => matches!(pointer.as_ref(),
                                                        ScalarExpression::Name { .. })
                                                        && aggregate::ordinary_value(inner) && **inner == output_types[position],
                                                    place @ ScalarPlace::Index { .. } => self.has_joined_root_reborrow
                                                        && self.exact_indexed_store_type(place, slot.destination, inner, &output_types[position]),
                                                    place => self.has_projected_child && aggregate::ordinary_value(&output_types[position])
                                                        && matches!(direct_checked_field_base(place),
                                                            Some((ScalarExpression::Name { .. }, ScalarFieldReference::Resolved(field)))
                                                                if matches!(inner.as_ref(), ScalarType::Struct(id) if id == &field.structure)),
                                                }))
                                {
                                    self.proof_points.checked_named_scalar_stores
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
                            FrozenInstall { rhs: rhs_point, install: self.cursor.expect("assignment install point") }
                        };
                        let FrozenInstall { rhs: rhs_point, install } = emitted;
                        if capture.is_none() && assignment.targets.len() == 1 && assignment.values.len() == 1
                            && position == 0 && aggregate::ordinary_value(&output_types[position])
                            && matches!(&slot.replacement, AssignmentReplacement::Resolved(ty) if ty == &output_types[position])
                        {
                            if let CfgPointKind::Assign { target, rhs_point: actual, .. } = &self.cfg.points[install.0].kind {
                                if *actual == rhs_point
                                    && self.proof_points.fallible_allocation_bindings.as_ref()
                                        .and_then(|sites| sites.get(&target.binding))
                                        .and_then(|owner| self.exact_guarded_index(
                                            &assignment.targets[position].place, slot.destination, &target.binding, owner))
                                        == Some(&output_types[position])
                                {
                                    self.proof_points.guarded_indexed_stores.insert(install);
                                }
                                if let (ScalarPlace::Field { base, field: ScalarFieldReference::Resolved(field), .. },
                                    [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(projection))]) =
                                    (&target.place, target.projections.as_slice())
                                {
                                    if let ScalarPlace::Name { span, .. } = base.as_ref() {
                                        let root = self.place_root(&target.place, slot.destination);
                                        if *actual == rhs_point && field == projection
                                            && matches!(self.binding_types.get(&target.binding),
                                                Some(ScalarType::Struct(id)) if *id == field.structure)
                                            && matches!(self.prepared_place_root(root.ordinal, *span)?,
                                                Some(PreparedPlaceRoot::Lexical(binding)) if binding == &target.binding)
                                        {
                                            if let Some(evaluation) = group.rhs_evaluations().next() {
                                                self.aggregate_facts.receipt_scalar_field_writes.insert(
                                                    install, (evaluation.expression, slot.destination),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Only the checked primary output can start same-array retention.
                        // Projected outputs have distinct points and do not inherit this proof.
                        if simple_destinations && position == 0
                            && self.aggregate_facts.array_calls.get(&rhs_point).is_some_and(|(producer, _, binding)| {
                                Some(*producer) == primary_array_producer
                                    && matches!(&self.cfg.points[rhs_point.0].kind, CfgPointKind::Call { output: 0, .. })
                                    && matches!(&self.cfg.points[install.0].kind,
                                        CfgPointKind::Assign { target, rhs_point: actual, .. }
                                            if *actual == rhs_point && target.projections.is_empty() && target.binding == *binding)
                            })
                        {
                            array_rebind = Some((install, slot.destination));
                        }
                        self.record_frozen_install(slot.destination, emitted);
                        let other_writer = item_index == 0
                            && self.aggregate_facts.literal_array_other_writer == Some(ordinal);
                        let literal_writer = (scope == ReferenceScopeId(0) && item_index == 1)
                            || (item_index == 0 && self.aggregate_facts.literal_array_writer == Some(ordinal));
                        if (literal_writer || other_writer) && existing_prepared
                            && position == 0 && assignment.targets.len() == 1 && assignment.values.len() == 1
                            && self.aggregate_facts.literal_array_bind.is_some()
                            && matches!(output_types[position], ScalarType::RuntimeArray { .. })
                        {
                            if let Some(evaluation) = group.rhs_evaluations().next() {
                                if let Some(argument) = self.literal_array_argument(evaluation.expression, rhs_point) {
                                    let write = Some(aggregate::LiteralArrayWrite {
                                        rhs: rhs_point,
                                        writer: install,
                                        producer: evaluation.expression,
                                        destination: slot.destination,
                                        argument,
                                    });
                                    if other_writer {
                                        self.aggregate_facts.literal_array_other_write = write;
                                    } else {
                                        self.aggregate_facts.literal_array_write = write;
                                    }
                                }
                            }
                        }
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
                    if let Some((install, destination)) = array_rebind.filter(|_| simple_destinations) {
                        self.aggregate_facts.array_installs.insert(install, destination);
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
        let mut return_bounds = None;
        if scope == ReferenceScopeId(0) {
            let backing_forward = self.checked_backing_forward(region, &result.output_points);
            let first_return_point = self.cfg.points.len();
            let mut value_points = result.output_points.iter();
            for slot in region.final_outputs() {
                return_bounds.get_or_insert((slot.span.start, slot.span.end)).1 = slot.span.end;
                let Some(&value_point) = value_points.next() else { continue };
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
            self.root_return_points = first_return_point..self.cfg.points.len();
        }
        let scope_exit = self.point(CfgPointKind::ScopeExit, region.span, scope, BTreeSet::new());
        if declares_guarded_owner {
            self.proof_points.guarded_allocation_scopes.insert(scope.0,
                super::control::GuardedAllocationScope { scope, entry: scope_entry, exit: scope_exit });
        }
        if self.aggregate_facts.needs_scope_exit_metadata() {
            self.aggregate_facts.scope_exits.insert(scope_exit, ordinal);
        }
        if let Some((start, end)) = return_bounds {
            let return_span = ByteSpan::new(start, end);
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
        let mut leaving = super::control::ScopeExitBindings::default();
        let prepared = self.proof_points.scope_exit_bindings.as_mut();
        if prepared.is_some() && scope == ReferenceScopeId(0) {
            for slot in &calls.boundary.parameters {
                leaving.insert(parameter_binding_identity(&self.source, self.function_span, block.span, slot));
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
            if prepared.is_some() {
                leaving.insert(id);
            }
        }
        let prepared = prepared.is_some();
        self.author_local_scope_exit(scope_exit, &leaving, prepared);
        if let Some(inventory) = self.proof_points.scope_exit_bindings.as_mut() {
            if !leaving.is_empty() {
                inventory.insert(scope_exit, leaving);
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Default, PartialEq)]
struct ReferenceEdgeState {
    aggregate: Option<aggregate::AggregateEdgeState>,
    bindings: BTreeMap<ReferenceBindingId, BTreeSet<ReferenceOriginId>>,
    projected: ReferenceProjectedRows,
    uninitialized: HashSet<ReferenceBindingId>,
    value: Option<BTreeSet<ReferenceOriginId>>,
    released: BTreeSet<ReferenceBindingId>,
    invalidated: BTreeSet<ReferenceBindingId>,
    replaced_array_extents: HashSet<ReferenceBindingId>,
    point_values: ReferencePointValues,
    return_outputs: BTreeMap<usize, BTreeSet<ReferenceOriginId>>,
    moved: Vec<(ReferencePlaceId, SourceSpan)>,
    guarded_whole_moves: BTreeMap<ReferenceBindingId, SourceSpan>,
    moved_checked_handles: BTreeMap<ReferenceBindingId, Vec<(Vec<ReferencePlaceProjection>, SourceSpan)>>,
    assignment_previous: BTreeMap<CfgPointId, BTreeSet<ReferenceOriginId>>,
}

impl ReferenceEdgeState {
    fn refine_allocation_edge(
        &mut self,
        edge: &super::control::GuardedAllocationEdge,
        successor: CfgPointId,
        enrolled_move_owner: bool,
    ) {
        if successor == edge.absent {
            self.uninitialized.insert(edge.binding.clone());
        } else if successor == edge.present
            && !self.released.contains(&edge.binding)
            && !self.invalidated.contains(&edge.binding)
            && if enrolled_move_owner { !self.guarded_whole_moves.contains_key(&edge.binding) }
                else { !self.moved.iter().any(|(place, _)| place.binding == edge.binding) }
        {
            self.uninitialized.remove(&edge.binding);
        }
    }

    fn expire_scope(
        &mut self,
        point: &ReferenceFlowPoint,
        inventory: Option<&HashMap<CfgPointId, super::control::ScopeExitBindings>>,
        facts: &[ReferenceOrigin],
    ) {
        let exiting = inventory.and_then(|exits| exits.get(&point.id));
        if inventory.is_some() && exiting.is_none() {
            return;
        }
        if let Some(binding) = exiting.and_then(super::control::ScopeExitBindings::singleton) {
            self.bindings.remove(binding);
            self.uninitialized.remove(binding);
            self.moved_checked_handles.remove(binding);
            self.guarded_whole_moves.remove(binding);
            self.released.remove(binding);
            self.invalidated.remove(binding);
            self.replaced_array_extents.remove(binding);
            self.moved.retain(|(place, _)| &place.binding != binding);
            self.projected.retain(|known| known.survives_scope(facts, |owner| owner != binding));
            return;
        }
        let live = |binding: &ReferenceBindingId| match exiting {
            Some(bindings) => !bindings.contains(binding),
            None => binding.block_span != point.source_span.range, // detached/unprepared only
        };
        self.bindings.retain(|binding, _| live(binding));
        self.uninitialized.retain(&live);
        self.moved.retain(|(place, _)| live(&place.binding));
        self.guarded_whole_moves.retain(|binding, _| live(binding));
        self.moved_checked_handles.retain(|binding, _| live(binding));
        self.released.retain(&live);
        self.invalidated.retain(&live);
        self.replaced_array_extents.retain(&live);
        self.projected.retain(|known| known.survives_scope(facts, &live));
    }
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

#[cfg(test)]
#[test]
fn guarded_allocation_facts_seal_distinct_polarities_in_branch_order() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 100);
    let binding = ReferenceBindingId {
        source, function_span: span, block_span: span,
        declaration_span: ByteSpan::new(10, 11), kind: ReferenceBindingKind::Declared,
    };
    let mut facts = CallablePointFacts::default();
    let [outer, inner] = [CfgPointId(1), CfgPointId(3)];
    facts.record_guarded_allocation(inner, OccurrenceOrdinal(0), binding.clone(), [CfgPointId(4), CfgPointId(6)], false);
    facts.record_guarded_allocation(outer, OccurrenceOrdinal(0), binding.clone(), [CfgPointId(2), CfgPointId(8)], true);
    facts.record_guarded_allocation(inner, OccurrenceOrdinal(0), binding.clone(), [CfgPointId(4), CfgPointId(6)], true);
    facts.record_guarded_allocation(CfgPointId(9), OccurrenceOrdinal(0), binding.clone(), [CfgPointId(10); 2], true);
    let [(first, first_edges), (second, second_edges)] = facts.guarded_allocation_edges.iter().collect::<Vec<_>>()[..] else {
        panic!("expected two distinct branch facts");
    };
    assert_eq!((*first, first_edges.present, first_edges.absent), (outer, CfgPointId(2), CfgPointId(8)));
    assert_eq!((*second, second_edges.present, second_edges.absent), (inner, CfgPointId(6), CfgPointId(4)));
    assert_eq!((&first_edges.binding, &second_edges.binding), (&binding, &binding));
}

#[cfg(test)]
#[test]
fn guarded_allocation_availability_refines_only_the_exact_successor_binding() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 100);
    let binding = |start| ReferenceBindingId {
        source: source.clone(), function_span: span, block_span: span,
        declaration_span: ByteSpan::new(start, start + 1), kind: ReferenceBindingKind::Declared,
    };
    let target = binding(10);
    let unrelated = binding(20);
    let edge = super::control::GuardedAllocationEdge {
        condition: OccurrenceOrdinal(0),
        binding: target.clone(), present: CfgPointId(3), absent: CfgPointId(2),
    };
    let mut entry = ReferenceEdgeState::default();
    entry.uninitialized.insert(unrelated.clone());
    let mut failure = entry.clone();
    failure.refine_allocation_edge(&edge, edge.absent, false);
    assert_eq!(failure.uninitialized, HashSet::from([target.clone(), unrelated.clone()]));
    let mut other = failure.clone();
    other.refine_allocation_edge(&edge, CfgPointId(4), false);
    assert!(other == failure);
    let mut success = failure.clone();
    success.refine_allocation_edge(&edge, edge.present, false);
    assert!(success == entry);
    assert!(failure.uninitialized.contains(&target));
    failure.released.insert(target.clone());
    let mut retired = failure.clone();
    retired.refine_allocation_edge(&edge, edge.present, false);
    assert!(retired == failure);

    failure.released.clear();
    let moved = ReferencePlaceId {
        source: source.clone(),
        function_span: span,
        declaration_span: target.declaration_span,
        block_span: span,
        binding: target.clone(),
        projections: Vec::new(),
        place: ScalarPlace::Name { name: "target".into(), span },
    };
    failure.moved.push((moved, SourceSpan::new(source, span)));
    let mut moved_edge = failure.clone();
    moved_edge.refine_allocation_edge(&edge, edge.present, false);
    assert!(moved_edge == failure);
    let mut indexed = failure.clone();
    indexed.guarded_whole_moves.insert(target.clone(), indexed.moved[0].1.clone());
    let mut indexed_edge = indexed.clone();
    indexed_edge.refine_allocation_edge(&edge, edge.present, true);
    assert!(indexed_edge == indexed);
    let mut other_owner = edge;
    other_owner.binding = unrelated.clone();
    indexed_edge.refine_allocation_edge(&other_owner, other_owner.present, true);
    assert!(indexed_edge.uninitialized.contains(&target));
    assert!(!indexed_edge.uninitialized.contains(&unrelated));
}

#[cfg(test)]
#[test]
fn prepared_scope_expiry_uses_exact_declared_and_introduced_bindings() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 100);
    let binding = |start| ReferenceBindingId {
        source: source.clone(), function_span: span, block_span: span,
        declaration_span: ByteSpan::new(start, start + 1), kind: ReferenceBindingKind::Declared,
    };
    let outer = binding(10);
    let inner = binding(20);
    let introduced = binding(30);
    let point = |id, scope| ReferenceFlowPoint {
        id: CfgPointId(id), kind: CfgPointKind::ScopeExit, source_span: SourceSpan::new(source.clone(), span),
        scope: ReferenceScopeId(scope), possible_origins: None, successors: Vec::new(),
    };
    let inner_exit = point(1, 1);
    let outer_exit = point(2, 0);
    let cohort = |bindings: &[&ReferenceBindingId]| {
        let mut result = super::control::ScopeExitBindings::default();
        for &binding in bindings {
            result.insert(binding.clone());
        }
        result
    };
    let inventory = HashMap::from([
        (inner_exit.id, cohort(&[&inner, &introduced])),
        (outer_exit.id, cohort(&[&outer])),
    ]);
    let mut different_span = binding(40);
    different_span.block_span = ByteSpan::new(1, 10);
    assert!(!inventory[&inner_exit.id].contains(&different_span));
    let mixed = cohort(&[&inner, &different_span]);
    assert!(mixed.contains(&inner) && mixed.contains(&different_span));
    assert!(!mixed.contains(&outer));
    let mut state = ReferenceEdgeState::default();
    for binding in [&outer, &inner, &introduced] {
        let place = ReferencePlaceId {
            source: source.clone(), function_span: span, block_span: span,
            declaration_span: binding.declaration_span, binding: binding.clone(),
            projections: vec![ReferencePlaceProjection::Index(ScalarExpression::Integer { value: 0.into(), span })],
            place: ScalarPlace::Name { name: String::new(), span },
        };
        state.bindings.insert(binding.clone(), BTreeSet::new());
        state.uninitialized.insert(binding.clone());
        state.moved.push((place.clone(), SourceSpan::new(source.clone(), span)));
        state.guarded_whole_moves.insert(binding.clone(), SourceSpan::new(source.clone(), span));
        state.moved_checked_handles.insert(binding.clone(), Vec::new());
        state.released.insert(binding.clone());
        state.invalidated.insert(binding.clone());
        state.projected.push(ReferenceProjectedState { retains_loan: false, place, origins: BTreeSet::new(), uninitialized: true });
    }
    state.expire_scope(&point(0, 2), Some(&inventory), &[]); // empty checked scope, not detached
    assert_eq!(state.bindings.len(), 3);
    let mut single = state.clone();
    single.expire_scope(&outer_exit, Some(&inventory), &[]);
    let others = BTreeSet::from([inner.clone(), introduced.clone()]);
    assert!(single.bindings.keys().eq(others.iter()));
    assert!(single.moved_checked_handles.keys().eq(others.iter()));
    assert!(single.guarded_whole_moves.keys().eq(others.iter()));
    assert_eq!(single.uninitialized, HashSet::from([inner.clone(), introduced.clone()]));
    assert_eq!(single.released, others);
    assert_eq!(single.invalidated, others);
    assert!(single.moved.iter().map(|(place, _)| &place.binding).eq([&inner, &introduced]));
    assert!(single.projected.iter().map(|known| &known.place.binding).eq([&inner, &introduced]));

    state.expire_scope(&inner_exit, Some(&inventory), &[]);
    assert_eq!(state.bindings, BTreeMap::from([(outer.clone(), BTreeSet::new())]));
    assert_eq!(state.uninitialized, HashSet::from([outer.clone()]));
    assert!(matches!(state.moved.as_slice(), [(place, _)] if place.binding == outer));
    assert_eq!(state.moved_checked_handles, BTreeMap::from([(outer.clone(), Vec::new())]));
    assert!(state.guarded_whole_moves.keys().eq([&outer]));
    assert_eq!(state.released, BTreeSet::from([outer.clone()]));
    assert_eq!(state.invalidated, BTreeSet::from([outer.clone()]));
    assert!(matches!(state.projected.as_slice(), [known] if known.place.binding == outer));
    state.expire_scope(&outer_exit, Some(&inventory), &[]);
    assert!(state == ReferenceEdgeState::default());
}

#[cfg(test)]
#[test]
fn prepared_loan_read_reachability_stops_only_at_the_exact_binding_exit() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let binding = |start| ReferenceBindingId {
        source: source.clone(), function_span: span, block_span: span,
        declaration_span: ByteSpan::new(start, start + 1), kind: ReferenceBindingKind::Declared,
    };
    let outer = binding(1);
    let inner = binding(2);
    let point = |id, scope, kind, successors| ReferenceFlowPoint {
        id: CfgPointId(id), kind, source_span: SourceSpan::new(source.clone(), span),
        scope: ReferenceScopeId(scope), possible_origins: None, successors,
    };
    let points = [
        point(0, 1, CfgPointKind::ScopeEnter, vec![CfgPointId(1)]),
        point(1, 1, CfgPointKind::ScopeExit, vec![CfgPointId(2)]),
        point(2, 0, CfgPointKind::Read { binding: outer.clone() }, vec![]),
    ];
    let mut cohort = super::control::ScopeExitBindings::default();
    cohort.insert(inner);
    let mut exact = HashMap::from([(CfgPointId(1), cohort)]);
    assert!(reference_read_reachable(&points, Some(&exact), true, CfgPointId(0), &outer));
    exact.get_mut(&CfgPointId(1)).unwrap().insert(outer.clone());
    assert!(!reference_read_reachable(&points, Some(&exact), true, CfgPointId(0), &outer));
    assert!(!reference_read_reachable(&points, None, false, CfgPointId(0), &outer));

    let mut branched = points.to_vec();
    branched[0].successors = vec![CfgPointId(2), CfgPointId(1)];
    assert!(reference_read_reachable(&branched, Some(&exact), true, CfgPointId(0), &outer));
    branched[1].successors = vec![CfgPointId(1)];
    assert!(reference_read_reachable(&branched, Some(&exact), false, CfgPointId(0), &outer));
    branched[1].kind = CfgPointKind::Join;
    assert!(reference_read_reachable(&branched, Some(&exact), false, CfgPointId(0), &outer));
    let mut pending = Vec::new();
    assert_eq!(defer_read_search_siblings(&[CfgPointId(2)], &mut pending), Some(CfgPointId(2)));
    assert_eq!(pending.capacity(), 0);
}

#[derive(Clone, Default, PartialEq)]
struct ReferenceProjectedRows {
    rows: Vec<ReferenceProjectedState>,
    live_loans: bool,
}

impl ReferenceProjectedRows {
    fn push(&mut self, row: ReferenceProjectedState) {
        self.live_loans |= row.retains_loan && !row.uninitialized;
        self.rows.push(row);
    }

    fn retain(&mut self, mut keep: impl FnMut(&ReferenceProjectedState) -> bool) {
        if self.live_loans {
            self.retain_mut(|row| keep(row));
        } else {
            self.rows.retain(keep);
        }
    }

    fn retain_mut(&mut self, mut keep: impl FnMut(&mut ReferenceProjectedState) -> bool) {
        let mut live_loans = false;
        self.rows.retain_mut(|row| {
            let retained = keep(row);
            live_loans |= retained && row.retains_loan && !row.uninitialized;
            retained
        });
        self.live_loans = live_loans;
    }

    fn loan_rows(&self) -> &[ReferenceProjectedState] {
        if self.live_loans { &self.rows } else { &[] }
    }
}

impl std::ops::Deref for ReferenceProjectedRows {
    type Target = Vec<ReferenceProjectedState>;

    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}

#[derive(Clone, PartialEq)]
struct ReferenceProjectedState {
    retains_loan: bool,
    place: ReferencePlaceId,
    origins: BTreeSet<ReferenceOriginId>,
    uninitialized: bool,
}

impl ReferenceProjectedState {
    fn survives_scope(
        &self,
        facts: &[ReferenceOrigin],
        live: impl Fn(&ReferenceBindingId) -> bool,
    ) -> bool {
        live(&self.place.binding) && !self.refers_to(facts, |origin| {
            declared_local_loan(origin)
                .is_some_and(|loan| !live(&loan.origin_place.binding))
        })
    }

    fn refers_to(
        &self,
        facts: &[ReferenceOrigin],
        affected: impl Fn(&ReferenceOrigin) -> bool,
    ) -> bool {
        self.origins.iter().any(|origin| affected(&facts[origin.0]))
    }

    fn refers_to_owner(
        &self,
        facts: &[ReferenceOrigin],
        affected: impl Fn(&ReferenceBindingId) -> bool,
    ) -> bool {
        self.refers_to(facts, |origin| match origin {
            ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                let mut current = Some(loan);
                while let Some(loan) = current {
                    if affected(&loan.origin_place.binding) {
                        return true;
                    }
                    current = loan.parent.as_deref();
                }
                false
            }
            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
        })
    }
}

fn declared_local_loan(origin: &ReferenceOrigin) -> Option<&ReferenceLoanId> {
    match origin {
        ReferenceOrigin::Fresh { loan, .. }
            if loan.origin_place.binding.kind == ReferenceBindingKind::Declared => Some(loan),
        _ => None,
    }
}

fn expired_local_loan_at(origin: &ReferenceOrigin, use_site: &SourceSpan) -> Option<SourceSpan> {
    let loan = declared_local_loan(origin)?;
    let owner = &loan.origin_place.binding;
    (owner.source == use_site.source
        && (use_site.range.start < owner.block_span.start
            || use_site.range.end > owner.block_span.end))
        .then(|| SourceSpan::new(owner.source.clone(), loan.creation_span))
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

fn available_place_replacement(place: &ReferencePlaceId) -> Option<(&ReferenceBindingId, Vec<ReferencePlaceProjection>)> {
    let last = place.projections.iter().rposition(|projection| matches!(projection, ReferencePlaceProjection::Dereference(_)))?;
    let ReferencePlaceProjection::Dereference(loan) = &place.projections[last] else { unreachable!() };
    let owner = &loan.origin_place;
    let (binding, mut projections) = available_place_replacement(owner)
        .unwrap_or_else(|| (&owner.binding, owner.projections.clone()));
    projections.extend_from_slice(&place.projections[last + 1..]);
    Some((binding, projections))
}

fn available_place(place: &ReferencePlaceId) -> ReferencePlaceId {
    available_place_for_check(place).into_owned()
}

fn available_place_for_check(place: &ReferencePlaceId) -> std::borrow::Cow<'_, ReferencePlaceId> {
    match available_place_replacement(place) {
        Some((binding, projections)) => std::borrow::Cow::Owned(ReferencePlaceId {
            binding: binding.clone(),
            projections,
            source: place.source.clone(),
            function_span: place.function_span,
            declaration_span: place.declaration_span,
            block_span: place.block_span,
            place: place.place.clone(),
        }),
        None => std::borrow::Cow::Borrowed(place),
    }
}

#[cfg(test)]
#[test]
fn read_only_place_normalization_borrows_only_unchanged_places() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let place = |start| {
        let declaration_span = ByteSpan::new(start, start + 1);
        ReferencePlaceId {
            source: source.clone(), function_span: span, block_span: span, declaration_span,
            binding: ReferenceBindingId {
                source: source.clone(), function_span: span, block_span: span, declaration_span,
                kind: ReferenceBindingKind::Declared,
            },
            place: ScalarPlace::Name { name: String::new(), span },
            projections: vec![ReferencePlaceProjection::Index(ScalarExpression::Integer { value: 0.into(), span })],
        }
    };
    let index = |value: u32| ReferencePlaceProjection::Index(ScalarExpression::Integer { value: value.into(), span });
    let deref = |origin_place| ReferencePlaceProjection::Dereference(Box::new(ReferenceLoanId {
        mode: ScalarReferenceMutability::Shared, origin_place, creation_span: span, parent: None,
    }));
    let first_owner = place(1);
    assert!(std::ptr::eq(available_place_for_check(&first_owner).as_ref(), &first_owner));

    let mut final_owner = place(4);
    final_owner.projections = vec![index(4)];
    let mut intermediate = place(3);
    intermediate.projections = vec![index(5), deref(final_owner.clone()), index(6)];
    let mut through = place(2);
    through.source = SourceIdentity::new("project".into(), "package".into(), "receiver".into(), "r2".into());
    through.function_span = ByteSpan::new(10, 20);
    through.block_span = ByteSpan::new(12, 18);
    through.declaration_span = ByteSpan::new(13, 14);
    through.binding.source = through.source.clone();
    through.binding.function_span = through.function_span;
    through.binding.block_span = through.block_span;
    through.binding.declaration_span = through.declaration_span;
    through.place = ScalarPlace::Name { name: "receiver".into(), span: through.declaration_span };
    through.projections = vec![index(7), deref(first_owner), index(8), deref(intermediate), index(9)];
    let resolved = available_place_for_check(&through);
    assert!(matches!(&resolved, std::borrow::Cow::Owned(_)));
    assert_eq!(resolved.as_ref(), &ReferencePlaceId {
        binding: final_owner.binding,
        projections: vec![index(4), index(6), index(9)],
        source: through.source.clone(),
        function_span: through.function_span,
        declaration_span: through.declaration_span,
        block_span: through.block_span,
        place: through.place.clone(),
    });
    let first = SourceSpan::new(through.source.clone(), ByteSpan::new(1, 2));
    let second = SourceSpan::new(through.source.clone(), ByteSpan::new(2, 3));
    let mut state = ReferenceEdgeState::default();
    state.moved.push((resolved.as_ref().clone(), first.clone()));
    state.moved.push((place(99), SourceSpan::new(through.source.clone(), span)));
    state.moved.push((resolved.as_ref().clone(), second.clone()));
    assert_eq!(unavailable_origins_for_normalized_place(&state, &resolved), vec![first, second]);
}

fn unavailable_place(state: &ReferenceEdgeState, place: &ReferencePlaceId) -> bool {
    let place = available_place_for_check(place);
    state
        .moved
        .iter()
        .any(|(moved, _)| reference_places_overlap(moved, &place))
}

fn unavailable_origins_for_normalized_place(state: &ReferenceEdgeState, place: &ReferencePlaceId) -> Vec<SourceSpan> {
    state
        .moved
        .iter()
        .filter(|(moved, _)| reference_places_overlap(moved, place))
        .map(|(_, origin)| origin.clone())
        .collect()
}

fn is_lexical_field_path(path: &[ReferencePlaceProjection]) -> bool {
    path.iter().all(|projection| {
        matches!(projection, ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(_)))
    })
}

fn direct_lexical_loan_root(place: &ReferencePlaceId) -> Option<&ScalarPlace> {
    let root = reference_root(&place.place);
    (matches!(root, ScalarPlace::Name { .. })
        && place.projections.iter().all(|projection| matches!(projection,
            ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(_))
                | ReferencePlaceProjection::Index(_))))
        .then_some(root)
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

fn defer_read_search_siblings(successors: &[CfgPointId], pending: &mut Vec<CfgPointId>) -> Option<CfgPointId> {
    let (last, siblings) = successors.split_last()?;
    pending.extend_from_slice(siblings);
    Some(*last)
}

fn lexical_read_kills(
    point: &ReferenceFlowPoint,
    scope_exits: Option<&HashMap<CfgPointId, super::control::ScopeExitBindings>>,
    binding: &ReferenceBindingId,
) -> bool {
    matches!(&point.kind, CfgPointKind::Assign { target, .. } if target.binding == *binding && target.projections.is_empty())
        || matches!(&point.kind, CfgPointKind::Bind { binding: bound } if bound == binding)
        || (matches!(&point.kind, CfgPointKind::ScopeExit) && match scope_exits {
            Some(exits) => exits.get(&point.id).is_some_and(|cohort| cohort.contains(binding)),
            None => point.source_span.range == binding.block_span,
        })
}

// The place/mode filter nominates candidates first. A forward branched body
// then resolves only the lexical holders actually queried, memoizing only CFG
// points reached by those queries. Each exact lexical identity has its own
// Read/Bind/overwrite/scope-exit cutoffs in this immutable graph.
#[derive(Default)]
struct LexicalConflictMemo {
    rows: HashMap<ReferenceBindingId, HashMap<CfgPointId, bool>>,
    retained: usize,
}

struct ConflictLiveness<'a> {
    points: &'a [ReferenceFlowPoint],
    scope_exits: Option<&'a HashMap<CfgPointId, super::control::ScopeExitBindings>>,
    strictly_forward: bool,
    lexical: Option<std::cell::RefCell<LexicalConflictMemo>>,
}

impl<'a> ConflictLiveness<'a> {
    fn new(
        points: &'a [ReferenceFlowPoint],
        scope_exits: Option<&'a HashMap<CfgPointId, super::control::ScopeExitBindings>>,
        strictly_forward: bool,
        branched: bool,
    ) -> Self {
        Self { points, scope_exits, strictly_forward,
            lexical: (strictly_forward && branched).then(Default::default) }
    }

    fn lexical_after(&self, from: CfgPointId, binding: &ReferenceBindingId) -> bool {
        let Some(lexical) = &self.lexical else {
            return reference_read_reachable(self.points, self.scope_exits, self.strictly_forward, from, binding);
        };
        let successors = &self.points[from.0].successors;
        if successors.is_empty() { return false; }
        // Keep enough room for a complete holder even on a wide graph, but
        // never retain the full holder × point relation.
        let limit = self.points.len().saturating_mul(2);
        let mut lexical = lexical.borrow_mut();
        let LexicalConflictMemo { rows, retained } = &mut *lexical;
        if !rows.contains_key(binding) {
            if *retained == limit {
                return reference_read_reachable(self.points, self.scope_exits, self.strictly_forward, from, binding);
            }
            rows.insert(binding.clone(), HashMap::new());
        }
        let known = rows.get_mut(binding).expect("indexed queried lexical holder");
        // A query which crosses the retention limit finishes the same walk.
        // Its remaining facts stay transient, so no visited suffix is rewalked.
        let mut transient = HashMap::new();
        // Keep the legacy traversal's last-successor-first preference.
        successors.iter().rev().any(|&start| {
            // Inclusive reachability for successors; the access itself is
            // never a use or a kill of the incoming holder value.
            if let Some(&live) = known.get(&start).or_else(|| transient.get(&start)) { return live; }
            let mut pending = vec![(start, 0usize)];
            while let Some(&(id, child)) = pending.last() {
                let point = &self.points[id.0];
                let live = if matches!(&point.kind, CfgPointKind::Read { binding: read } if read == binding) {
                    true
                } else if lexical_read_kills(point, self.scope_exits, binding) {
                    false
                } else if let Some(&successor) = point.successors.iter().rev().nth(child) {
                    debug_assert!(successor > id, "only strictly forward graphs use this refinement");
                    match known.get(&successor).or_else(|| transient.get(&successor)) {
                        Some(true) => true,
                        Some(false) => {
                            pending.last_mut().expect("current refinement frame").1 += 1;
                            continue;
                        }
                        None => {
                            pending.push((successor, 0));
                            continue;
                        }
                    }
                } else {
                    false
                };
                if *retained < limit {
                    known.insert(id, live);
                    *retained += 1;
                } else {
                    transient.insert(id, live);
                }
                pending.pop();
            }
            known.get(&start).or_else(|| transient.get(&start)).copied()
                .expect("refined successor fact")
        })
    }
}

#[cfg(test)]
#[test]
fn forward_conflict_liveness_memoizes_exact_holder_cutoffs() {
    let source = SourceIdentity::new("project".into(), "package".into(), "liveness.w".into(), "r1".into());
    let block = ByteSpan::new(0, 100);
    let binding = |start| ReferenceBindingId {
        source: source.clone(), function_span: block, block_span: block,
        declaration_span: ByteSpan::new(start, start + 1), kind: ReferenceBindingKind::Declared,
    };
    let holder = binding(1);
    let other = binding(2);
    let make = |index, kind, successors: &[usize]| ReferenceFlowPoint {
        id: CfgPointId(index), kind, source_span: SourceSpan::new(source.clone(), block),
        scope: ReferenceScopeId(0), possible_origins: None,
        successors: successors.iter().map(|&next| CfgPointId(next)).collect(),
    };
    let points = [
        make(0, CfgPointKind::Entry, &[1, 2]),
        make(1, CfgPointKind::ScopeExit, &[3]),
        make(2, CfgPointKind::Bind { binding: holder.clone() }, &[3]),
        make(3, CfgPointKind::Read { binding: holder.clone() }, &[4]),
        make(4, CfgPointKind::Read { binding: other.clone() }, &[]),
    ];
    let mut exiting = super::control::ScopeExitBindings::default();
    exiting.insert(holder.clone());
    let exits = HashMap::from([(CfgPointId(1), exiting)]);
    let liveness = ConflictLiveness::new(&points, Some(&exits), true, true);
    let cache = liveness.lexical.as_ref().unwrap();
    assert!(cache.borrow().rows.is_empty(), "no work without a lexical conflict candidate");
    assert!(!liveness.lexical_after(CfgPointId(0), &holder));
    assert_eq!(cache.borrow().rows[&holder].len(), 2, "both kills stop the refinement");
    assert!(!liveness.lexical_after(CfgPointId(0), &holder));
    assert_eq!(cache.borrow().rows[&holder].len(), 2, "repeated query reuses exactly the prior facts");
    assert!(liveness.lexical_after(CfgPointId(1), &holder), "query starts after the site itself");
    assert!(liveness.lexical_after(CfgPointId(0), &other), "a different lexical identity is not killed");
    for id in 0..points.len() {
        for binding in [&holder, &other] {
            assert_eq!(liveness.lexical_after(CfgPointId(id), binding),
                reference_read_reachable(&points, Some(&exits), true, CfgPointId(id), binding));
        }
    }
}

#[cfg(test)]
#[test]
fn forward_conflict_liveness_retains_only_a_linear_budget_across_holders() {
    let source = SourceIdentity::new("project".into(), "package".into(), "budget.w".into(), "r1".into());
    let block = ByteSpan::new(0, 40);
    let holders: Vec<_> = (0..6).map(|index| ReferenceBindingId {
        source: source.clone(), function_span: block, block_span: block,
        declaration_span: ByteSpan::new(index, index + 1), kind: ReferenceBindingKind::Declared,
    }).collect();
    let points: Vec<_> = (0usize..16).map(|id| ReferenceFlowPoint {
        id: CfgPointId(id),
        kind: match id {
            5 => CfgPointKind::Read { binding: holders[4].clone() },
            9 => CfgPointKind::Read { binding: holders[2].clone() },
            _ => CfgPointKind::Entry,
        },
        source_span: SourceSpan::new(source.clone(), block), scope: ReferenceScopeId(0),
        possible_origins: None,
        successors: match id {
            0 => vec![CfgPointId(1), CfgPointId(2)],
            1 => vec![CfgPointId(3)],
            3 => vec![CfgPointId(4), CfgPointId(5)],
            15 => Vec::new(),
            id => vec![CfgPointId(id + 1)],
        },
    }).collect();
    let liveness = ConflictLiveness::new(&points, None, true, true);
    let cache = liveness.lexical.as_ref().unwrap();
    for binding in &holders {
        for id in [0, 4, 9, 12] {
            assert_eq!(liveness.lexical_after(CfgPointId(id), binding),
                reference_read_reachable(&points, None, true, CfgPointId(id), binding));
        }
    }
    let memo = cache.borrow();
    assert_eq!(memo.retained, points.len() * 2);
    assert_eq!(memo.retained, memo.rows.values().map(HashMap::len).sum::<usize>());
    assert_eq!(memo.rows.len(), 3, "later unrelated holders do not get retained rows");
    assert_eq!(memo.rows[&holders[0]].len(), points.len() - 1, "first holder keeps its whole useful suffix");
    assert!(!memo.rows[&holders[2]].contains_key(&CfgPointId(1)), "over-budget tail is not retained");
    drop(memo);
    assert!(!liveness.lexical_after(CfgPointId(0), &holders[0]));
    assert_eq!(cache.borrow().retained, points.len() * 2);

    let ordered = ConflictLiveness::new(&points, None, true, true);
    assert!(ordered.lexical_after(CfgPointId(0), &holders[4]));
    let memo = ordered.lexical.as_ref().unwrap().borrow();
    assert_eq!(memo.rows[&holders[4]], HashMap::from([
        (CfgPointId(2), true), (CfgPointId(3), true), (CfgPointId(5), true),
    ]), "last successor is explored first at both the seed and nested fork");
}

fn reference_read_reachable(
    points: &[ReferenceFlowPoint],
    scope_exits: Option<&HashMap<CfgPointId, super::control::ScopeExitBindings>>,
    strictly_forward: bool,
    from: CfgPointId,
    binding: &ReferenceBindingId,
) -> bool {
    reference_holder_read_reachable(points, scope_exits, strictly_forward, from, binding, None)
}

fn reference_holder_read_reachable(
    points: &[ReferenceFlowPoint],
    scope_exits: Option<&HashMap<CfgPointId, super::control::ScopeExitBindings>>,
    strictly_forward: bool,
    from: CfgPointId,
    binding: &ReferenceBindingId,
    projected: Option<&ReferencePlaceId>,
) -> bool {
    let mut pending = Vec::new();
    let successors = &points[from.0].successors;
    let mut track_visited = !strictly_forward || successors.len() > 1;
    let mut next = defer_read_search_siblings(successors, &mut pending);
    let mut visited = BTreeSet::new();
    while let Some(id) = next.take().or_else(|| pending.pop()) {
        if track_visited && !visited.insert(id.0) {
            continue;
        }
        if matches!(&points[id.0].kind, CfgPointKind::Read { binding: read } if read == binding) {
            return true;
        }
        if let Some(projected) = projected {
            match &points[id.0].kind {
                CfgPointKind::ReadPlace { candidates, pointer_point, reads_origin, .. }
                    if (pointer_point.is_some() && candidates.is_empty()
                        && (*reads_origin || points[id.0].possible_origins.is_none()))
                        || candidates.iter().any(|candidate| reference_places_overlap(candidate, projected)) =>
                {
                    return true;
                }
                CfgPointKind::Borrow { place, .. } | CfgPointKind::Move { place, .. }
                    if reference_places_overlap(place, projected) =>
                {
                    return true;
                }
                CfgPointKind::Assign { target, .. }
                    if exact_projected_prefix(target, projected) =>
                {
                    continue;
                }
                _ => {}
            }
        }
        if lexical_read_kills(&points[id.0], scope_exits, binding) {
            continue;
        }
        let successors = &points[id.0].successors;
        track_visited |= successors.len() > 1;
        next = defer_read_search_siblings(successors, &mut pending);
    }
    false
}

fn exact_projected_prefix(prefix: &ReferencePlaceId, place: &ReferencePlaceId) -> bool {
    prefix.binding == place.binding
        && prefix.projections.len() <= place.projections.len()
        && prefix.projections.iter().zip(&place.projections).all(|(a, b)| match (a, b) {
            (ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(a)),
             ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(b))) => a == b,
            (ReferencePlaceProjection::Index(ScalarExpression::Integer { value: a, .. }),
             ReferencePlaceProjection::Index(ScalarExpression::Integer { value: b, .. })) => a == b,
            _ => false,
        })
}


fn loan_borrows_place(
    loan: &ReferenceLoanId,
    place: &ReferencePlaceId,
    mode: Option<ScalarReferenceMutability>,
) -> bool {
    let mut candidate = Some(loan);
    while let Some(current) = candidate {
        if mode.is_none_or(|mode| current.mode == mode)
            && reference_places_overlap(&available_place_for_check(&current.origin_place), place)
        {
            return true;
        }
        candidate = current.parent.as_deref();
    }
    false
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

fn exact_projected_field_of_mutable_parent(loan: &ReferenceLoanId) -> Option<(&ReferenceLoanId, &ScalarStructFieldId)> {
    let parent = loan.parent.as_deref()?;
    if parent.mode != ScalarReferenceMutability::Mutable
        || parent.parent.is_some() || !parent.origin_place.projections.is_empty()
        || !matches!(parent.origin_place.place, ScalarPlace::Name { .. })
        || parent.origin_place.binding != loan.origin_place.binding
    {
        return None;
    }
    let Some((ScalarExpression::Name { .. }, ScalarFieldReference::Resolved(field))) =
        direct_checked_field_base(&loan.origin_place.place)
    else { return None };
    let [ReferencePlaceProjection::Dereference(projected_parent),
        ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(projected_field))] =
        loan.origin_place.projections.as_slice()
    else { return None };
    (projected_parent.as_ref() == parent && projected_field == field).then_some((parent, field))
}

#[derive(Clone, Copy)]
enum ProjectedWriteAdmission<'a> {
    Known(&'a BTreeSet<ReferenceOriginId>),
    Joined { issuance: ByteSpan, mode: ScalarReferenceMutability, index: Option<&'a ScalarExpression> },
}

#[derive(Clone, Copy)]
struct DisjointProjectedWrite<'a> {
    admission: ProjectedWriteAdmission<'a>,
    parent: &'a ReferenceLoanId,
    field: &'a ScalarStructFieldId,
}

impl DisjointProjectedWrite<'_> {
    fn excludes(self, origin: ReferenceOriginId, loan: &ReferenceLoanId) -> bool {
        let admitted = match self.admission {
            ProjectedWriteAdmission::Known(origins) => origins.contains(&origin),
            ProjectedWriteAdmission::Joined { issuance, mode, index } => {
                if loan.mode != mode || loan.creation_span != issuance {
                    return false;
                }
                if let Some(ScalarExpression::Integer { value: written, .. }) = index {
                    let [ReferencePlaceProjection::Dereference(root),
                        ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)),
                        ReferencePlaceProjection::Index(ScalarExpression::Integer { value: borrowed, .. })] =
                        loan.origin_place.projections.as_slice() else { return false };
                    return root.as_ref() == self.parent && loan.parent.as_deref() == Some(self.parent)
                        && loan.origin_place.binding == self.parent.origin_place.binding
                        && field == self.field && written != borrowed;
                }
                index.is_none()
            },
        };
        admitted && exact_projected_field_of_mutable_parent(loan).is_some_and(|(parent, field)|
            parent == self.parent && field.structure == self.field.structure && field.index != self.field.index)
    }
}

#[derive(Clone, Copy)]
enum LoanAccess<'a> {
    Direct,
    DirectShared,
    Through(ReferenceOriginId),
    WriteThrough(&'a InvalidationCandidate, Option<DisjointProjectedWrite<'a>>),
    ReadThrough(&'a ReferenceLoanId),
    Borrow(&'a ReferenceLoanId),
}

fn conflicting_loan<'a>(
    facts: &'a [ReferenceOrigin],
    origin: ReferenceOriginId,
    place: &ReferencePlaceId,
    access: LoanAccess<'_>,
) -> Option<&'a ReferenceLoanId> {
    if match access {
        LoanAccess::Direct | LoanAccess::DirectShared | LoanAccess::ReadThrough(_) | LoanAccess::Borrow(_) => false,
        LoanAccess::Through(selected) => origin == selected,
        LoanAccess::WriteThrough(selected, _) => origin == selected.origin,
    } {
        return None;
    }
    let loan = match &facts[origin.0] {
        ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
        _ => return None,
    };
    (match access {
        LoanAccess::ReadThrough(selected) => {
            loan.mode == ScalarReferenceMutability::Mutable
                && loan_has_ancestor(loan, selected)
                && reference_places_overlap(&available_place_for_check(&loan.origin_place), place)
        }
        LoanAccess::WriteThrough(selected, projected) => {
            !projected.is_some_and(|projected| projected.excludes(origin, loan))
                && loan_borrows_place(loan, place, None)
                && !loan_has_ancestor(&selected.loan, loan)
        }
        LoanAccess::Borrow(new) => {
            (new.mode == ScalarReferenceMutability::Mutable
                || loan.mode == ScalarReferenceMutability::Mutable)
                && reference_places_overlap(&available_place_for_check(&loan.origin_place), place)
                && !loan_has_ancestor(new, loan)
        }
        LoanAccess::DirectShared => loan_borrows_place(loan, place, Some(ScalarReferenceMutability::Mutable)),
        LoanAccess::Direct | LoanAccess::Through(_) => loan_borrows_place(loan, place, None),
    }).then_some(loan)
}

fn retire_allocation_loans(
    state: &mut ReferenceEdgeState, facts: &[ReferenceOrigin], owns: impl Fn(&ReferenceBindingId) -> bool,
) {
    for origins in state.bindings.values_mut() {
        origins.retain(|origin| match &facts[origin.0] {
            ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } =>
                !owns(&loan.origin_place.binding),
            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => true,
        });
    }
    state.projected.retain(|known| !owns(&known.place.binding)
        && !known.refers_to_owner(facts, &owns));
}

#[cfg(test)]
#[test]
fn release_uses_the_exact_allocation_loan_cohort() {
    let source = SourceIdentity::new("project".into(), "package".into(), "cfg.w".into(), "r1".into());
    let block = ByteSpan::new(0, 100);
    let binding = |start| ReferenceBindingId {
        source: source.clone(), function_span: block, block_span: block,
        declaration_span: ByteSpan::new(start, start + 1), kind: ReferenceBindingKind::Declared,
    };
    let first = binding(1);
    let second = binding(2);
    let retained = binding(3);
    let unaffected = binding(4);
    let place = |owner: &ReferenceBindingId| ReferencePlaceId {
        source: source.clone(), function_span: block, block_span: block,
        declaration_span: owner.declaration_span, binding: owner.clone(),
        place: ScalarPlace::Name { name: String::new(), span: owner.declaration_span },
        projections: Vec::new(),
    };
    let origin = |owner: &ReferenceBindingId, index: u32, start| {
        let span = ByteSpan::new(start, start + 1);
        let mut origin_place = place(owner);
        let offset = ScalarExpression::Integer { value: index.into(), span };
        origin_place.place = ScalarPlace::Index {
            base: Box::new(origin_place.place), index: Box::new(offset.clone()), span, index_span: span,
        };
        origin_place.projections.push(ReferencePlaceProjection::Index(offset));
        ReferenceOrigin::Fresh {
            allocation: ReturnedAllocationId {
                source: source.clone(), function_span: block,
                declaration_span: owner.declaration_span, creation_span: owner.declaration_span,
            },
            loan: ReferenceLoanId {
                mode: ScalarReferenceMutability::Shared, origin_place, creation_span: span, parent: None,
            },
        }
    };
    let facts = [origin(&first, 0, 10), origin(&first, 1, 11), origin(&second, 1, 12)];
    let point = |id, kind, successors| ReferenceFlowPoint {
        id: CfgPointId(id), kind, source_span: SourceSpan::new(source.clone(), block),
        scope: ReferenceScopeId(0), possible_origins: None, successors,
    };
    let points = [
        point(0, CfgPointKind::Entry, vec![CfgPointId(1)]),
        point(1, CfgPointKind::Read { binding: retained.clone() }, vec![CfgPointId(2)]),
        point(2, CfgPointKind::Read { binding: unaffected.clone() }, Vec::new()),
    ];
    let mut state = ReferenceEdgeState::default();
    state.bindings.insert(retained.clone(), BTreeSet::from([ReferenceOriginId(0), ReferenceOriginId(1)]));
    state.bindings.insert(unaffected.clone(), BTreeSet::from([ReferenceOriginId(2)]));
    let live = ConflictLiveness::new(&points, None, true, false);
    let conflicts = conflicting_live_loan(&live, &facts, &state, CfgPointId(0), &place(&first), LoanAccess::Direct);
    assert_eq!(conflicts, [
        SourceSpan::new(source.clone(), ByteSpan::new(10, 11)),
        SourceSpan::new(source, ByteSpan::new(11, 12)),
    ]);
    retire_allocation_loans(&mut state, &facts, |owner| owner == &first);
    assert!(state.bindings[&retained].is_empty());
    assert_eq!(state.bindings[&unaffected], BTreeSet::from([ReferenceOriginId(2)]));
}

fn conflicting_live_loan(
    liveness: &ConflictLiveness<'_>,
    facts: &[ReferenceOrigin],
    state: &ReferenceEdgeState,
    point: CfgPointId,
    place: &ReferencePlaceId,
    access: LoanAccess<'_>,
) -> Vec<SourceSpan> {
    let place = available_place_for_check(place);
    let mut conflicts = Vec::new();
    for (binding, origins) in &state.bindings {
        let mut reachable = false;
        for origin in origins {
            let Some(loan) = conflicting_loan(facts, *origin, &place, access) else {
                continue;
            };
            if !reachable {
                if !liveness.lexical_after(point, binding) {
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
    for row in state.projected.loan_rows() {
        if !row.retains_loan || row.uninitialized {
            continue;
        }
        let mut reachable = None;
        for origin in &row.origins {
            if let Some(loan) = conflicting_loan(facts, *origin, &place, access) {
                if declared_local_loan(&facts[origin.0]).is_some()
                    && !*reachable.get_or_insert_with(|| reference_holder_read_reachable(
                        liveness.points, liveness.scope_exits, liveness.strictly_forward,
                        point, &row.place.binding, Some(&row.place),
                    ))
                {
                    continue;
                }
                if !conflicts.iter().any(|span| span.source == loan.origin_place.source
                    && span.range == loan.creation_span) {
                    conflicts.push(SourceSpan::new(loan.origin_place.source.clone(), loan.creation_span));
                }
            }
        }
    }
    conflicts
}

#[cfg(test)]
#[test]
fn selected_holder_keeps_projected_only_shared_loan_live_until_replaced() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let place = |start| {
        let declaration_span = ByteSpan::new(start, start + 1);
        let binding = ReferenceBindingId {
            source: source.clone(), function_span: span, block_span: span,
            declaration_span, kind: ReferenceBindingKind::Declared,
        };
        ReferencePlaceId {
            source: source.clone(), function_span: span, block_span: span, declaration_span,
            binding, projections: Vec::new(), place: ScalarPlace::Name { name: String::new(), span },
        }
    };
    let referent = place(1);
    let mut slot = place(2);
    slot.projections.push(ReferencePlaceProjection::Index(
        ScalarExpression::Integer { value: 0.into(), span },
    ));
    let holder = SelectedIndexedHolder {
        holder: slot.binding.clone(), selector: place(3).binding, sites: HashSet::new(),
    };
    let creation_span = ByteSpan::new(4, 5);
    let unrelated = place(6);
    let loan = |origin_place| ReferenceLoanId {
        mode: ScalarReferenceMutability::Shared, origin_place, creation_span, parent: None,
    };
    let facts = [
        ReferenceOrigin::BorrowedFrom { parameter: 0, loan: loan(referent.clone()) },
        ReferenceOrigin::BorrowedFrom { parameter: 1, loan: loan(unrelated.clone()) },
        ReferenceOrigin::Null { span },
    ];
    let mut state = ReferenceEdgeState::default();
    holder.write(&mut state, &slot, &BTreeSet::from([ReferenceOriginId(0)]), &facts);
    assert!(state.bindings.is_empty());
    let conflicts = |state: &ReferenceEdgeState, place: &ReferencePlaceId, access: LoanAccess<'_>| {
        conflicting_live_loan(&ConflictLiveness::new(&[], None, true, false),
            &facts, state, CfgPointId(0), place, access)
    };
    assert_eq!(conflicts(&state, &referent, LoanAccess::Direct),
        vec![SourceSpan::new(source, creation_span)]);
    assert!(conflicts(&state, &referent, LoanAccess::DirectShared).is_empty());
    assert!(conflicts(&state, &unrelated, LoanAccess::Direct).is_empty());
    holder.write(&mut state, &slot, &BTreeSet::from([ReferenceOriginId(1)]), &facts);
    assert!(conflicts(&state, &referent, LoanAccess::Direct).is_empty());
    assert_eq!(conflicts(&state, &unrelated, LoanAccess::Direct).len(), 1);
    let exit = ReferenceFlowPoint {
        id: CfgPointId(0), kind: CfgPointKind::ScopeExit,
        source_span: SourceSpan::new(referent.source.clone(), span),
        scope: ReferenceScopeId(0), possible_origins: None, successors: vec![],
    };
    let mut cohort = super::control::ScopeExitBindings::default();
    cohort.insert(slot.binding.clone());
    state.expire_scope(&exit, Some(&HashMap::from([(exit.id, cohort)])), &facts);
    assert!(!state.projected.live_loans);
    assert!(conflicts(&state, &unrelated, LoanAccess::Direct).is_empty());
    holder.write(&mut state, &slot, &BTreeSet::from([ReferenceOriginId(2)]), &facts);
    assert!(!state.projected.is_empty());
    assert!(!state.projected.live_loans);
    assert!(state.projected.loan_rows().is_empty());
}

#[derive(Default)]
pub(super) struct SelectedIndexedHolders {
    by_holder: HashMap<ReferenceBindingId, SelectedIndexedHolder>,
    selectors: HashSet<ReferenceBindingId>,
}

impl SelectedIndexedHolders {
    fn record(&mut self, holder: &ReferenceBindingId, selector: &ReferenceBindingId, span: ByteSpan) {
        if let Some(selected) = self.by_holder.get_mut(holder) {
            if selected.selector == *selector {
                selected.sites.insert(span);
            }
        } else {
            self.selectors.insert(selector.clone());
            self.by_holder.insert(holder.clone(), SelectedIndexedHolder {
                holder: holder.clone(), selector: selector.clone(), sites: HashSet::from([span]),
            });
        }
    }

    fn disabled_by<'a>(&self, point: &'a ReferenceFlowPoint) -> Option<&'a ReferenceBindingId> {
        match &point.kind {
            CfgPointKind::Borrow { place, .. } if self.by_holder.contains_key(&place.binding)
                || self.selectors.contains(&place.binding) => Some(&place.binding),
            CfgPointKind::Assign { target, .. } if self.selectors.contains(&target.binding) => Some(&target.binding),
            _ => None,
        }
    }
}

pub(super) struct SelectedIndexedHolder {
    holder: ReferenceBindingId,
    selector: ReferenceBindingId,
    sites: HashSet<ByteSpan>,
}

impl SelectedIndexedHolder {
    fn selected(&self, place: &ReferencePlaceId) -> bool {
        place.binding == self.holder
            && matches!(&place.place, ScalarPlace::Index { span, .. } if self.sites.contains(span))
            && matches!(place.projections.as_slice(), [ReferencePlaceProjection::Index(ScalarExpression::Name { .. })])
    }

    fn constant(&self, place: &ReferencePlaceId) -> bool {
        place.binding == self.holder
            && matches!(place.projections.as_slice(), [ReferencePlaceProjection::Index(ScalarExpression::Integer { .. })])
    }

    fn eligible_origins(origins: &BTreeSet<ReferenceOriginId>, facts: &[ReferenceOrigin]) -> bool {
        !origins.is_empty() && origins.iter().all(|origin| match &facts[origin.0] {
            ReferenceOrigin::Null { .. } => true,
            ReferenceOrigin::BorrowedFrom { loan, .. } =>
                loan.mode == ScalarReferenceMutability::Shared && loan.parent.is_none(),
            _ => false,
        })
    }

    fn write(
        &self, state: &mut ReferenceEdgeState, target: &ReferencePlaceId,
        origins: &BTreeSet<ReferenceOriginId>, facts: &[ReferenceOrigin],
    ) {
        let selected = self.selected(target);
        let holds_loan = |origins: &BTreeSet<ReferenceOriginId>| origins.iter()
            .any(|origin| matches!(facts[origin.0], ReferenceOrigin::BorrowedFrom { .. }));
        let retained_loan = holds_loan(origins);
        state.projected.retain_mut(|known| {
            if !reference_places_overlap(&known.place, target) {
                return true;
            }
            if if selected { self.constant(&known.place) } else { self.selected(&known.place) } {
                if known.uninitialized {
                    return true;
                }
                if !Self::eligible_origins(&known.origins, facts) {
                    return false;
                }
                known.retains_loan |= retained_loan || holds_loan(&known.origins);
                known.origins.extend(origins);
                true
            } else {
                false
            }
        });
        state.projected.push(ReferenceProjectedState {
            retains_loan: retained_loan,
            place: target.clone(), origins: origins.clone(), uninitialized: false,
        });
    }
}

fn extend_projected_read_origins(
    state: &ReferenceEdgeState,
    candidate: &ReferencePlaceId,
    read: &mut BTreeSet<ReferenceOriginId>,
    holder: Option<&SelectedIndexedHolder>,
) -> bool {
    let mut found = false;
    let selected = holder.filter(|holder| holder.selected(candidate));
    for known in state.projected.iter() {
        if !known.uninitialized && (selected.is_some_and(|holder| holder.selected(&known.place))
            || reference_places_same(&known.place, candidate)) {
            found = true;
            read.extend(&known.origins);
        }
    }
    found
}

#[cfg(test)]
#[test]
fn projected_read_keeps_presence_separate_from_origin_contents() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let place = |start| {
        let declaration_span = ByteSpan::new(start, start + 1);
        ReferencePlaceId {
            source: source.clone(), function_span: span, block_span: span, declaration_span,
            binding: ReferenceBindingId {
                source: source.clone(), function_span: span, block_span: span, declaration_span,
                kind: ReferenceBindingKind::Declared,
            },
            place: ScalarPlace::Name { name: String::new(), span },
            projections: vec![ReferencePlaceProjection::Index(ScalarExpression::Integer { value: 0.into(), span })],
        }
    };
    let checked = place(1);
    let unrelated = place(2);
    let mut state = ReferenceEdgeState::default();
    state.projected.push(ReferenceProjectedState {
        retains_loan: false,
        place: checked.clone(), origins: BTreeSet::from([ReferenceOriginId(9)]), uninitialized: true,
    });
    state.projected.push(ReferenceProjectedState {
        retains_loan: false,
        place: unrelated, origins: BTreeSet::from([ReferenceOriginId(8)]), uninitialized: false,
    });
    let mut read = BTreeSet::new();
    assert!(!extend_projected_read_origins(&state, &checked, &mut read, None));
    state.projected.push(ReferenceProjectedState {
        retains_loan: false,
        place: checked.clone(), origins: BTreeSet::new(), uninitialized: false,
    });
    assert!(extend_projected_read_origins(&state, &checked, &mut read, None));
    assert!(read.is_empty());
    state.projected.rows.last_mut().unwrap().origins.insert(ReferenceOriginId(1));
    assert!(extend_projected_read_origins(&state, &checked, &mut read, None));
    assert_eq!(read, BTreeSet::from([ReferenceOriginId(1)]));
}

fn retire_overwritten_binding_origins(
    state: &mut ReferenceEdgeState,
    previous_origins: &BTreeSet<ReferenceOriginId>,
) {
    if previous_origins.is_empty() {
        return;
    }
    for origins in state.bindings.values_mut() {
        origins.retain(|origin| !previous_origins.contains(origin));
    }
}

fn definite_assignment_candidate<'a>(
    candidates: &'a [InvalidationCandidate],
    original_origins: Option<&BTreeSet<ReferenceOriginId>>,
) -> Option<&'a InvalidationCandidate> {
    let [candidate] = candidates else { return None };
    (candidate.loan.mode == ScalarReferenceMutability::Mutable
        && original_origins.is_some_and(|origins|
            origins.len() == 1 && origins.contains(&candidate.origin)))
        .then_some(candidate)
}

fn restore_definitely_written_place(state: &mut ReferenceEdgeState, target: &ReferencePlaceId) {
    if !state.moved_checked_handles.is_empty() {
        reinitialize_checked_handles(state, target, CheckedHandleStorage::Pointee);
    }
    state.moved.retain(|(moved, _)| !exact_projected_prefix(target, moved));
}

fn update_projected_assignment<'a>(
    state: &mut ReferenceEdgeState,
    mut targets: impl ExactSizeIterator<Item = (&'a ReferencePlaceId, bool)> + Clone,
    origins: Option<&BTreeSet<ReferenceOriginId>>,
    unique_target: bool,
    installs_local_loan: bool,
) {
    if targets.len() == 0 {
        return;
    }
    if unique_target {
        debug_assert_eq!(targets.len(), 1);
        let (target, can_retain_loan) = targets.next().unwrap();
        if let Some(origins) = origins {
            write_projected_reference(state, target, origins, installs_local_loan && can_retain_loan
                && target.binding.kind == ReferenceBindingKind::Declared
                && !target.projections.is_empty()
                && exact_projected_prefix(target, target));
        } else {
            state.projected.retain(|known| !reference_places_overlap(&known.place, target));
        }
        return;
    }
    state.projected.retain_mut(|known| {
        let mut exact = false;
        for (target, can_retain_loan) in targets.clone() {
            if reference_places_overlap(&known.place, target) {
                if !reference_places_same(&known.place, target) || (installs_local_loan && !can_retain_loan) {
                    return false;
                }
                exact = true;
            }
        }
        if !exact {
            return true;
        }
        let Some(origins) = origins else { return false };
        if known.uninitialized
            || known.place.binding.kind != ReferenceBindingKind::Declared
            || known.place.projections.is_empty()
            || !exact_projected_prefix(&known.place, &known.place)
        {
            return false;
        }
        known.retains_loan |= installs_local_loan;
        known.origins.extend(origins);
        true
    });
}

#[cfg(test)]
#[test]
fn overwritten_binding_origins_retire_only_exact_previous_facts() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let binding = |start| ReferenceBindingId {
        source: source.clone(), function_span: span, block_span: span,
        declaration_span: ByteSpan::new(start, start + 1), kind: ReferenceBindingKind::Declared,
    };
    let first = binding(1);
    let alias = binding(2);
    let untouched = ReferenceOriginId(0);
    let retired = ReferenceOriginId(1);
    let mut state = ReferenceEdgeState::default();
    state.bindings.insert(first.clone(), BTreeSet::from([untouched, retired]));
    state.bindings.insert(alias.clone(), BTreeSet::from([retired]));
    let original = state.bindings.clone();
    retire_overwritten_binding_origins(&mut state, &BTreeSet::new());
    assert_eq!(state.bindings, original);
    retire_overwritten_binding_origins(&mut state, &BTreeSet::from([retired]));
    assert_eq!(state.bindings, BTreeMap::from([
        (first, BTreeSet::from([untouched])), (alias, BTreeSet::new()),
    ]));
}

#[cfg(test)]
#[test]
fn projected_assignment_preserves_alternatives_only_for_exact_initialized_slots() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let place = |start, indices: &[u32]| {
        let declaration_span = ByteSpan::new(start, start + 1);
        ReferencePlaceId {
            source: source.clone(), function_span: span, block_span: span, declaration_span,
            binding: ReferenceBindingId {
                source: source.clone(), function_span: span, block_span: span, declaration_span,
                kind: ReferenceBindingKind::Declared,
            },
            place: ScalarPlace::Name { name: String::new(), span },
            projections: indices.iter().map(|index| ReferencePlaceProjection::Index(
                ScalarExpression::Integer { value: (*index).into(), span },
            )).collect(),
        }
    };
    let root = place(1, &[]);
    let projected = place(1, &[0]);
    let missing = place(1, &[1]);
    let unrelated = place(2, &[]);
    let candidate = |origin, place: &ReferencePlaceId| InvalidationCandidate {
        origin: ReferenceOriginId(origin),
        place: place.clone(),
        loan: ReferenceLoanId {
            mode: ScalarReferenceMutability::Mutable,
            origin_place: place.clone(),
            creation_span: span,
            parent: None,
        },
    };
    let one = BTreeSet::from([ReferenceOriginId(0)]);
    let two = BTreeSet::from([ReferenceOriginId(0), ReferenceOriginId(1)]);
    let mut availability = ReferenceEdgeState::default();
    let descendant = place(1, &[0, 2]);
    let sibling_descendant = place(1, &[1, 2]);
    let container = place(1, &[]);
    for moved in [&container, &projected, &descendant, &missing, &sibling_descendant] {
        availability.moved.push((moved.clone(), SourceSpan::new(source.clone(), span)));
    }
    for (candidates, original) in [
        (vec![candidate(0, &projected), candidate(1, &missing)], &two),
        (vec![candidate(0, &projected)], &two),
    ] {
        if let Some(definite) = definite_assignment_candidate(&candidates, Some(original)) {
            restore_definitely_written_place(&mut availability, &definite.place);
        }
        assert_eq!(availability.moved.len(), 5);
    }
    let mut same_slot = projected.clone();
    let [ReferencePlaceProjection::Index(ScalarExpression::Integer { span: replacement_span, .. })] =
        same_slot.projections.as_mut_slice() else { unreachable!() };
    *replacement_span = ByteSpan::new(4, 5);
    let single_candidate = [candidate(0, &same_slot)];
    let definite = definite_assignment_candidate(&single_candidate, Some(&one)).unwrap();
    restore_definitely_written_place(&mut availability, &definite.place);
    assert!(availability.moved.iter().map(|(place, _)| place)
        .eq([&container, &missing, &sibling_descendant]));

    let previous = BTreeSet::from([ReferenceOriginId(0)]);
    let replacement = BTreeSet::from([ReferenceOriginId(1)]);
    let mut state = ReferenceEdgeState::default();
    write_projected_reference(&mut state, &projected, &previous, true);
    write_projected_reference(&mut state, &unrelated, &previous, false);

    let mut single = state.clone();
    let capacity = single.projected.capacity();
    update_projected_assignment(&mut single, [(&root, true)].into_iter(), Some(&replacement), true, false);
    assert_eq!(single.projected.capacity(), capacity);
    assert!(matches!(single.projected.as_slice(), [first, last]
        if first.place == unrelated && first.origins == previous
        && last.place == root && last.origins == replacement));
    assert!(!single.projected.live_loans);

    let mut filtered_single = state.clone();
    update_projected_assignment(&mut filtered_single, [(&projected, true)].into_iter(),
        Some(&replacement), false, false);
    assert_eq!(filtered_single.projected[0].origins,
        BTreeSet::from([ReferenceOriginId(0), ReferenceOriginId(1)]));
    assert!(filtered_single.projected.live_loans);

    let mut weak = state.clone();
    update_projected_assignment(&mut weak, [(&projected, true), (&missing, true)].into_iter(),
        Some(&replacement), false, false);
    assert!(matches!(weak.projected.as_slice(), [first, last]
        if first.place == projected && first.retains_loan
        && first.origins == BTreeSet::from([ReferenceOriginId(0), ReferenceOriginId(1)])
        && last.place == unrelated && last.origins == previous));
    assert!(weak.projected.live_loans);

    let mut uninitialized = state.clone();
    uninitialized.projected.rows[0].uninitialized = true;
    update_projected_assignment(&mut uninitialized, [(&projected, true), (&missing, true)].into_iter(),
        Some(&replacement), false, true);
    assert_eq!(uninitialized.projected.len(), 1);
    assert_eq!(uninitialized.projected[0].place, unrelated);
    assert!(!uninitialized.projected.live_loans);

    let mut mixed_candidates = state.clone();
    write_projected_reference(&mut mixed_candidates, &projected, &previous, false);
    write_projected_reference(&mut mixed_candidates, &missing, &previous, false);
    update_projected_assignment(&mut mixed_candidates,
        [(&projected, true), (&missing, false)].into_iter(), Some(&replacement), false, true);
    assert_eq!(mixed_candidates.projected.len(), 2);
    assert!(mixed_candidates.projected.iter().all(|row| row.place != missing));
    let retained = mixed_candidates.projected.iter().find(|row| row.place == projected).unwrap();
    assert!(retained.retains_loan);
    assert_eq!(retained.origins, BTreeSet::from([ReferenceOriginId(0), ReferenceOriginId(1)]));
    assert!(mixed_candidates.projected.live_loans);

    let mut without_rhs = state.clone();
    update_projected_assignment(&mut without_rhs, [(&projected, true), (&missing, true)].into_iter(),
        None, false, false);
    assert!(matches!(without_rhs.projected.as_slice(), [first]
        if first.place == unrelated && first.origins == previous));

    update_projected_assignment(&mut state, [(&root, true), (&projected, true)].into_iter(),
        Some(&replacement), false, false);
    assert!(matches!(state.projected.as_slice(), [first]
        if first.place == unrelated && first.origins == previous));
    assert!(!state.projected.live_loans);
}

fn write_projected_reference(
    state: &mut ReferenceEdgeState,
    place: &ReferencePlaceId,
    origins: &BTreeSet<ReferenceOriginId>,
    retains_loan: bool,
) {
    state
        .projected
        .retain(|known| !reference_places_overlap(&known.place, place));
    state.projected.push(ReferenceProjectedState {
        retains_loan,
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
        // Retire the same resolved place that the assignment checked for
        // conflicts. A child's raw dereference projection is not the first
        // field of the owner, and would overlap every sibling syntactically.
        let target = available_place_for_check(target);
        state
            .bindings
            .values()
            .flat_map(|origins| origins.iter().copied())
            .filter(|origin| match &facts[origin.0] {
                ReferenceOrigin::Fresh { loan, .. }
                | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                    loan_borrows_place(loan, &target, None)
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
    projected: Option<DisjointProjectedWrite<'_>>,
) -> BTreeSet<ReferenceOriginId> {
    if candidates.is_empty() {
        return BTreeSet::new();
    }
    state
        .bindings
        .values()
        .flat_map(|origins| origins.iter().copied())
        .filter(|origin| match &facts[origin.0] {
            ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                if projected.is_some_and(|projected| projected.excludes(*origin, loan)) {
                    return false;
                }
                let replaces = |candidate: &InvalidationCandidate| {
                    reference_places_overlap(&loan.origin_place, &candidate.place)
                        && !loan_has_ancestor(&candidate.loan, loan)
                };
                match candidates {
                    [candidate] => candidate.origin != *origin && replaces(candidate),
                    _ => !candidates.iter().any(|candidate| candidate.origin == *origin)
                        && candidates.iter().any(replaces),
                }
            }
            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
        })
        .collect()
}

#[cfg(test)]
#[test]
fn previous_candidate_origins_preserve_exact_selected_loan_exclusions() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let place = |start| {
        let declaration_span = ByteSpan::new(start, start + 1);
        ReferencePlaceId {
            source: source.clone(), function_span: span, block_span: span, declaration_span,
            binding: ReferenceBindingId {
                source: source.clone(), function_span: span, block_span: span, declaration_span,
                kind: ReferenceBindingKind::Declared,
            },
            projections: Vec::new(), place: ScalarPlace::Name { name: String::new(), span },
        }
    };
    let loan = |origin_place| ReferenceLoanId {
        mode: ScalarReferenceMutability::Mutable, origin_place, creation_span: span, parent: None,
    };
    let root = place(1);
    let unrelated = place(2);
    let facts = [
        ReferenceOrigin::BorrowedFrom { parameter: 0, loan: loan(root.clone()) },
        ReferenceOrigin::BorrowedFrom { parameter: 1, loan: loan(root.clone()) },
        ReferenceOrigin::BorrowedFrom { parameter: 2, loan: loan(root.clone()) },
        ReferenceOrigin::BorrowedFrom { parameter: 3, loan: loan(unrelated.clone()) },
    ];
    let candidates = [
        InvalidationCandidate { origin: ReferenceOriginId(0), place: root.clone(), loan: loan(root) },
        InvalidationCandidate { origin: ReferenceOriginId(1), place: unrelated.clone(), loan: loan(unrelated) },
    ];
    let mut state = ReferenceEdgeState::default();
    state.bindings.insert(place(3).binding, BTreeSet::from([
        ReferenceOriginId(0), ReferenceOriginId(1), ReferenceOriginId(2), ReferenceOriginId(3),
    ]));
    assert!(previous_candidate_origins(&state, &facts, &[], None).is_empty());
    assert_eq!(previous_candidate_origins(&state, &facts, &candidates[..1], None),
        BTreeSet::from([ReferenceOriginId(1), ReferenceOriginId(2)]));
    assert_eq!(previous_candidate_origins(&state, &facts, &candidates, None),
        BTreeSet::from([ReferenceOriginId(2), ReferenceOriginId(3)]));
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
    local_borrow: Option<&'a local_borrow::Sealed>,
    initial_points: &'a [ReferenceFlowPoint],
    has_grouped_assignment_capture: Option<bool>,
    forward_initial_graph: Option<super::control::ForwardInitialGraph>,
    forward_graph_out: Option<&'a mut Option<super::control::ForwardInitialGraph>>,
    scope_exit_bindings: Option<&'a HashMap<CfgPointId, super::control::ScopeExitBindings>>,
    pending_named_reborrow: Option<&'a super::control::PendingNamedReborrow>,
    resolve_pending_named_reborrow: bool,
    indexed_holders: Option<&'a SelectedIndexedHolders>,
    raw_address_exposures: &'a [(CfgPointId, Option<ReferenceBindingId>)],
    points: &'a mut [ReferenceFlowPoint],
    facts: &'a mut Vec<ReferenceOrigin>,
    provisional_origin_index: Option<&'a mut ReferenceOriginIndex>,
    allocations: &'a BTreeSet<ReferenceBindingId>,
    checked_join_values: &'a [(CfgPointId, [Option<CfgPointId>; 2])],
    checked_block_values: &'a [(CfgPointId, CfgPointId)],
    array_length_owners: Option<&'a HashSet<ReferenceBindingId>>,
    pending_returned_extent: Option<CfgPointId>,
    widened_rebind_points: Option<&'a HashSet<CfgPointId>>,
    fallible_allocation_bindings: Option<&'a HashMap<ReferenceBindingId, super::control::GuardedAllocationBinding>>,
    fallible_aliases: Option<&'a HashMap<ReferenceBindingId, ReferenceBindingId>>,
    guarded_allocation_edges: Option<&'a BTreeMap<CfgPointId, super::control::GuardedAllocationEdge>>,
    guarded_indexed_stores: Option<&'a HashSet<CfgPointId>>,
    guarded_runtime_moves: Option<&'a BTreeMap<CfgPointId, GuardedRuntimeMove>>,
    guarded_releases: Option<&'a BTreeMap<CfgPointId, ReferenceBindingId>>,
    checked_binding_initializers: &'a [(CfgPointId, CfgPointId)],
    resolved_checked_call_outputs: &'a [CfgPointId],
    checked_pointee_reads: &'a [CfgPointId],
    joined_reborrow_descendants: &'a [CfgPointId],
    joined_shared_aliases: &'a [(CfgPointId, CfgPointId)],
    exclusive_scalar_write_moves: &'a [(CfgPointId, CfgPointId, CfgPointId)],
    shared_scalar_read_calls: &'a [(CfgPointId, CfgPointId)],
    joined_shared_return_calls: &'a [(CfgPointId, CfgPointId)],
    joined_checked_releases: &'a [CfgPointId],
    checked_named_scalar_stores: &'a [CfgPointId],
    named_reborrow_parents: &'a [super::control::NamedReborrowParent],
}

impl<'a> ReferenceTransferCfg<'a> {
    fn new(cfg: &'a mut ScalarReferenceCfg, proof: Option<&'a CallablePointFacts>) -> Self {
        Self {
            local_borrow: None,
            initial_points: &cfg.initial_points,
            has_grouped_assignment_capture: proof.and_then(|proof| proof.has_grouped_assignment_capture),
            forward_initial_graph: None,
            forward_graph_out: None,
            scope_exit_bindings: proof.and_then(|proof| proof.scope_exit_bindings.as_ref()),
            pending_named_reborrow: proof.and_then(|proof| proof.pending_named_reborrow.as_ref()),
            resolve_pending_named_reborrow: false,
            indexed_holders: proof.and_then(|proof| proof.indexed_holders.as_deref()),
            raw_address_exposures: proof.map_or(&[], |proof| &proof.raw_address_exposures),
            points: &mut cfg.points,
            facts: &mut cfg.facts,
            provisional_origin_index: None,
            allocations: &cfg.allocations,
            checked_join_values: proof.map_or(&[], |proof| &proof.checked_join_values),
            checked_block_values: proof.map_or(&[], |proof| &proof.checked_block_values),
            pending_returned_extent: proof.and_then(|proof| proof.pending_returned_extent),
            widened_rebind_points: proof.and_then(|proof| proof.widened_rebind_points.as_ref()),
            array_length_owners: proof.map(|proof| &proof.array_length_owners).filter(|owners| !owners.is_empty()),
            fallible_allocation_bindings: proof.and_then(|proof| proof.fallible_allocation_bindings.as_ref()),
            fallible_aliases: proof.map(|proof| &proof.fallible_aliases).filter(|aliases| !aliases.is_empty()),
            guarded_allocation_edges: proof.map(|proof| &proof.guarded_allocation_edges),
            guarded_indexed_stores: proof.map(|proof| &proof.guarded_indexed_stores),
            guarded_runtime_moves: proof.map(|proof| &proof.guarded_runtime_moves).filter(|moves| !moves.is_empty()),
            guarded_releases: proof.map(|proof| &proof.guarded_releases).filter(|releases| !releases.is_empty()),
            checked_binding_initializers: proof.map_or(&[], |proof| &proof.checked_binding_initializers),
            resolved_checked_call_outputs: proof.map_or(&[], |proof| &proof.resolved_checked_call_outputs),
            checked_pointee_reads: proof.map_or(&[], |proof| &proof.checked_pointee_reads),
            joined_reborrow_descendants: proof.map_or(&[], |proof| &proof.joined_reborrow_descendants),
            joined_shared_aliases: proof.map_or(&[], |proof| &proof.joined_shared_aliases),
            exclusive_scalar_write_moves: proof.map_or(&[], |proof| &proof.exclusive_scalar_write_moves),
            shared_scalar_read_calls: proof.map_or(&[], |proof| &proof.shared_scalar_read_calls),
            joined_shared_return_calls: proof.map_or(&[], |proof| &proof.joined_shared_return_calls),
            joined_checked_releases: proof.map_or(&[], |proof| &proof.joined_checked_releases),
            checked_named_scalar_stores: proof.map_or(&[], |proof| &proof.checked_named_scalar_stores),
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

fn shared_scalar_read_effect(
    source: &SourceIdentity, calls: &FunctionCallSites, ordinal: OccurrenceOrdinal,
    target: &ResolvedCallTarget, effects: Option<&HashMap<ByteSpan, SharedScalarReadEffect>>,
) -> Option<SharedScalarReadEffect> {
    if matches!(target, ResolvedCallTarget::LocalCallable(_)) {
        return (calls.local_shared_call_families.get(&ordinal) == Some(&true))
            .then_some(SharedScalarReadEffect { parameter: 0, descriptor: false, joined_only: false, bool_prefix: false, returns_formal: false });
    }
    let ResolvedCallTarget::ModuleCallable {
        source: callee, declaration_span, concrete: ConcreteCallSelection::Declaration,
    } = target else { return None };
    let effect = if callee == source {
        *effects?.get(declaration_span)?
    } else {
        let (identity, effect) = calls.imported_shared_readers.get(&ordinal)?;
        if &identity.source != callee || identity.range != *declaration_span {
            return None;
        }
        *effect
    };
    (!effect.descriptor).then_some(effect)
}

fn exact_exclusive_scalar_writer(function: &ScalarFunction, calls: &FunctionCallSites) -> bool {
    (|| {
        if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() { return None; }
        let [value, pointer] = calls.boundary.parameters.as_slice() else { return None };
        let ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner } = &pointer.ty
            else { return None };
        if !aggregate::ordinary_value(inner) || &value.ty != inner.as_ref()
            || !calls.boundary.root_outputs.iter().all(|output| output.ty == ScalarType::Unit)
            || calls.boundary.root_outputs.len() > 1
        { return None; }
        let root = calls.structure.root;
        let [item] = calls.structure.blocks.get(root.0)?.items.as_slice() else { return None };
        let BlockItemKind::Assignment { group } = item.kind else { return None };
        let assignment = calls.assignments.get(&(group.block, group.item))?;
        let evaluations = assignment.rhs_evaluations();
        let destinations = assignment.destinations();
        let ([rhs], [target]) = (evaluations.as_slice(), destinations.as_slice()) else { return None };
        let OccurrenceNode::Expression(address) = *calls.structure.edges.get(&(
            OccurrenceParent::Place(target.destination), OccurrenceEdge::DereferencePointer,
        ))? else { return None };
        let formal = |occurrence, parameter: &ParameterSlot| matches!(calls.prepared_plain_name(occurrence),
            Some(PlainNameIdentity::Lexical(binding)) if binding.kind == ReferenceBindingKind::Declared
                && binding.declaration_span == parameter.span);
        (assignment.projected_outputs().len() == 0
            && calls.structure.checked_types.places.get(&target.destination) == Some(inner.as_ref())
            && calls.structure.checked_types.expressions.get(&address) == Some(&pointer.ty)
            && calls.structure.checked_types.expressions.get(&rhs.expression) == Some(&value.ty)
            && formal(address, pointer) && formal(rhs.expression, value)).then_some(())
    })().is_some()
}

fn attach_local_exclusive_writers(calls: &mut FunctionCallSites, source: &SourceIdentity, writers: &HashSet<ByteSpan>) {
    if writers.is_empty() { return; }
    calls.exclusive_scalar_writers = calls.call_occurrences.iter().filter_map(|call| {
        let ResolvedCallTarget::ModuleCallable { source: target, declaration_span, concrete: ConcreteCallSelection::Declaration } =
            &calls.sites.get(call.site)?.target else { return None };
        (target == source && writers.contains(declaration_span)
            && !calls.declared_unsafe_calls.contains(&call.ordinal.0)).then_some(call.ordinal)
    }).collect();
    for arm in &mut calls.arms {
        attach_local_exclusive_writers(arm, source, writers);
    }
}

fn joined_reborrow_calls_are_supported(
    source: &SourceIdentity, calls: &FunctionCallSites,
    effects: Option<&HashMap<ByteSpan, SharedScalarReadEffect>>,
) -> bool {
    let mut checked_release = false;
    calls.call_occurrences.iter().all(|call| {
        let target = &calls.sites[call.site].target;
        if *target == ResolvedCallTarget::Core(CoreOperationId::Free) {
            if std::mem::replace(&mut checked_release, true) {
                return false;
            }
            let Some(OccurrenceNode::Expression(argument)) = calls.structure.edges.get(&(
                OccurrenceParent::Expression(call.ordinal), OccurrenceEdge::CallArgument(0),
            )) else { return false };
            return !calls.structure.edges.contains_key(&(
                OccurrenceParent::Expression(call.ordinal), OccurrenceEdge::CallArgument(1),
            )) && matches!(calls.prepared_plain_name(*argument), Some(PlainNameIdentity::Lexical(_)))
                && matches!(calls.structure.checked_types.expressions.get(argument),
                    Some(ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner })
                        if aggregate::ordinary_value(inner));
        }
        if calls.declared_unsafe_calls.contains(&call.ordinal.0) {
            return false;
        }
        if calls.exclusive_scalar_writers.contains(&call.ordinal)
            || shared_scalar_read_effect(source, calls, call.ordinal, target, effects).is_some() {
            return true;
        }
        matches!(target,
            ResolvedCallTarget::ModuleCallable { source: callee, concrete: ConcreteCallSelection::Declaration, .. }
                if callee == source)
            && calls.checked_call_signatures.get(&call.ordinal).is_some_and(|signature|
                signature.parameters.iter().all(aggregate::ordinary_value)
                    && match signature.outputs.outputs.as_slice() {
                        [] => true,
                        [output] => output.ty == ScalarType::Unit || aggregate::ordinary_value(&output.ty),
                        _ => false,
                    })
    })
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

fn exact_reborrow_read_edge<'a>(
    read: &'a ReferenceFlowPoint,
    borrow: &ReferenceFlowPoint,
) -> Option<&'a ReferenceBindingId> {
    if read.id >= borrow.id || read.scope != borrow.scope || read.successors.as_slice() != [borrow.id] {
        return None;
    }
    let CfgPointKind::Read { binding } = &read.kind else { return None };
    Some(binding)
}

fn exact_pending_reborrow_read(
    read: &ReferenceFlowPoint,
    borrow: &ReferenceFlowPoint,
    binding: &ReferenceBindingId,
) -> bool {
    exact_reborrow_read_edge(read, borrow) == Some(binding)
}

#[cfg(test)]
#[test]
fn prepared_deferred_reborrow_requires_the_exact_read_edge() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let binding = ReferenceBindingId {
        source: source.clone(), function_span: span, block_span: span,
        declaration_span: ByteSpan::new(1, 2), kind: ReferenceBindingKind::Declared,
    };
    let substitute = ReferenceBindingId { declaration_span: ByteSpan::new(2, 3), ..binding.clone() };
    let borrow = ReferenceFlowPoint {
        id: CfgPointId(2), kind: CfgPointKind::Evaluate(ScalarExpression::Boolean { value: true, span }),
        source_span: SourceSpan::new(source, span), scope: ReferenceScopeId(1),
        possible_origins: None, successors: Vec::new(),
    };
    let mut read = ReferenceFlowPoint {
        id: CfgPointId(1), kind: CfgPointKind::Read { binding: binding.clone() },
        successors: vec![borrow.id], ..borrow.clone()
    };
    assert!(exact_pending_reborrow_read(&read, &borrow, &binding));
    assert!(!exact_pending_reborrow_read(&read, &borrow, &substitute));
    read.scope = ReferenceScopeId(2);
    assert!(!exact_pending_reborrow_read(&read, &borrow, &binding));
    read.scope = borrow.scope;
    read.successors.push(CfgPointId(3));
    assert!(!exact_pending_reborrow_read(&read, &borrow, &binding));
    read.successors = vec![CfgPointId(3)];
    assert!(!exact_pending_reborrow_read(&read, &borrow, &binding));
}

fn contains_loan_site(binding: &ReferenceBindingId, point: &ReferenceFlowPoint) -> bool {
    let site = point.source_span.range;
    binding.block_span.start <= site.start && site.end <= binding.block_span.end
}

fn deferred_local_reborrow_differs_in_scope(
    pending: &super::control::PendingNamedReborrow,
    point: &ReferenceFlowPoint,
    points: &[ReferenceFlowPoint],
) -> bool {
    matches!(&pending.root, super::control::DeferredReborrowRoot::Local { borrow, .. }
        if point.scope != points[borrow.0].scope)
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


struct JoinedReborrowShape<'a> {
    parent: &'a ReferenceBindingId,
    holders: HashMap<&'a ReferenceBindingId, CfgPointId>,
    descendants: BTreeMap<CfgPointId, (CfgPointId, &'a ReferenceBindingId)>,
    returned_aliases: BTreeMap<CfgPointId, (&'a ReferenceBindingId, CfgPointId)>,
    nested_exit: Option<&'a super::control::ScopeExitBindings>,
}

fn joined_root_reborrow_binding<'a>(
    points: &'a [ReferenceFlowPoint],
    point: &ReferenceFlowPoint,
    initializers: &[(CfgPointId, CfgPointId)],
    pointee_reads: &[CfgPointId],
    aliases: &[(CfgPointId, CfgPointId)],
    descendant_sites: &[CfgPointId],
    read_calls: &[(CfgPointId, CfgPointId)],
    write_moves: &[(CfgPointId, CfgPointId, CfgPointId)],
    return_calls: &[(CfgPointId, CfgPointId)],
    checked_releases: &[CfgPointId],
    scope_exits: Option<&'a HashMap<CfgPointId, super::control::ScopeExitBindings>>,
    scalar_stores: &[CfgPointId],
    grouped_installs: &BTreeSet<CfgPointId>,
    exposures: &[(CfgPointId, Option<ReferenceBindingId>)],
) -> Option<JoinedReborrowShape<'a>> {
    let CfgPointKind::ReborrowThrough { mode, place, pointer_point } = &point.kind else { return None };
    let pointer = match place {
        ScalarPlace::Dereference { pointer, .. } => pointer.as_ref(),
        _ => {
            let (field, index) = joined_projection_field(place);
            if index.is_some_and(|index| !matches!(index, ScalarExpression::Integer { .. })) {
                return None;
            }
            match direct_checked_field_base(field)? {
                (pointer, ScalarFieldReference::Resolved(_)) => pointer,
                _ => return None,
            }
        },
    };
    let CfgPointKind::Read { binding } = &points.get(pointer_point.0)?.kind else { return None };
    let ScalarExpression::Name { span, .. } = pointer else { return None };
    if !exact_pending_reborrow_read(&points[pointer_point.0], point, binding)
        || points[pointer_point.0].source_span.range != *span
        || !exposures.is_empty()
    {
        return None;
    }
    let mutable = *mode == ScalarReferenceMutability::Mutable;
    let carrier_move = match checked_releases {
        [] => None,
        [release] if *release > point.id => {
            let release = points.get(release.0)?;
            let CfgPointKind::Release { target: ReleaseTarget::Checked { address_point, address_span, .. } } =
                &release.kind else { return None };
            let address = points.get(address_point.0)?;
            let [moved] = address.successors.as_slice() else { return None };
            let moved = points.get(moved.0)?;
            if address.id <= point.id || moved.id <= address.id || moved.id >= release.id
                || address.scope != release.scope || moved.scope != release.scope
                || &address.source_span != address_span || &moved.source_span != address_span
                || moved.successors.as_slice() != [release.id]
                || !contains_loan_site(binding, address) || !contains_loan_site(binding, release)
                || !matches!(&address.kind, CfgPointKind::Read { binding: selected } if selected == binding)
                || !matches!(&moved.kind, CfgPointKind::Move { owner, place,
                    ty: ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner } }
                    if owner == binding && &place.binding == binding && place.projections.is_empty()
                        && matches!(place.place, ScalarPlace::Name { .. }) && aggregate::ordinary_value(inner))
            { return None; }
            Some(moved.id)
        }
        _ => return None,
    };
    let [bind] = point.successors.as_slice() else { return None };
    let child = points.get(bind.0)?;
    let CfgPointKind::Bind { binding: child_binding } = &child.kind else { return None };
    if child.scope != point.scope || child_binding == binding
        || initializers.binary_search_by_key(bind, |&(bind, _)| bind)
            .ok().is_none_or(|index| initializers[index].1 != point.id)
    {
        return None;
    }
    // The forward shape pass checks every lexical holder use, including
    // a first use in a branch. Do not require the Bind to be followed by
    // a straight-line first observation before those facts can be used.
    let mut observed_root = false;
    let mut holders = HashMap::from([(child_binding, point.id)]);
    let mut descendants = BTreeMap::new();
    let mut returned_aliases = BTreeMap::new();
    let mut alias_count = 0;
    let mut exclusive_descendant = false;
    let nested = point.scope != ReferenceScopeId(0);
    let mut nested_exit = None;
    let mut next_pointee = &pointee_reads[pointee_reads.partition_point(|id| *id <= child.id)..];
    for rest in points.get(child.id.0 + 1..)? {
        match &rest.kind {
            CfgPointKind::ScopeExit if nested && rest.scope == point.scope => {
                let cohort = scope_exits?.get(&rest.id)?;
                if !cohort.contains(binding) || !cohort.contains(child_binding)
                    || nested_exit.replace(cohort).is_some()
                { return None; }
            }
            CfgPointKind::Read { binding: holder } if holders.contains_key(holder) => {
                let [deref] = rest.successors.as_slice() else { return None };
                let deref = points.get(deref.0)?;
                // A typed observation may execute in a nested branch, but its
                // holder evaluation and operation must share one lexical scope.
                if rest.scope != deref.scope || !contains_loan_site(holder, rest)
                    || !contains_loan_site(holder, deref)
                {
                    return None;
                }
                let birth = holders[holder];
                observed_root |= holder == child_binding;
                let writable = matches!(points.get(birth.0)?.kind, CfgPointKind::ReborrowThrough {
                    mode: ScalarReferenceMutability::Mutable, ..
                });
                if let CfgPointKind::ReborrowThrough {
                    mode, place: ScalarPlace::Dereference { pointer, .. }, pointer_point,
                } = &deref.kind
                {
                    if *mode == ScalarReferenceMutability::Mutable {
                        if !mutable || birth != point.id || exclusive_descendant {
                            return None;
                        }
                        exclusive_descendant = true;
                    } else if writable && birth != point.id {
                        return None;
                    }
                    if descendant_sites.binary_search(&deref.id).is_err()
                        || *pointer_point != rest.id || !matches!(pointer.as_ref(), ScalarExpression::Name { span, .. }
                            if *span == rest.source_span.range)
                    { return None; }
                    let [bind] = deref.successors.as_slice() else { return None };
                    let bound = points.get(bind.0)?;
                    let CfgPointKind::Bind { binding: grandchild } = &bound.kind else { return None };
                    if bound.scope != deref.scope || !contains_loan_site(grandchild, bound)
                        || grandchild == binding || holders.contains_key(grandchild)
                        || initializers.binary_search(&(*bind, deref.id)).is_err()
                    { return None; }
                    if descendants.insert(deref.id, (birth, holder)).is_some() { return None; }
                    holders.insert(grandchild, deref.id);
                    continue;
                }
                if let CfgPointKind::Bind { binding: alias } = &deref.kind {
                    if rest.scope != child.scope || writable || aliases.binary_search(&(deref.id, rest.id)).is_err()
                        || alias == binding || holders.contains_key(alias) || alias_count == MAX_JOINED_REBORROW_ALIASES
                    {
                        return None;
                    }
                    holders.insert(alias, birth);
                    alias_count += 1;
                    continue;
                }
                if !writable && return_calls.binary_search(&(deref.id, rest.id)).is_ok() {
                    let CfgPointKind::Call { output: 0, argument_points, .. } = &deref.kind else { return None };
                    let [bind] = deref.successors.as_slice() else { return None };
                    let bound = points.get(bind.0)?;
                    let CfgPointKind::Bind { binding: alias } = &bound.kind else { return None };
                    if argument_points.as_slice() != [rest.id] || bound.scope != deref.scope
                        || !contains_loan_site(alias, bound) || alias == binding || holders.contains_key(alias)
                        || initializers.binary_search(&(*bind, deref.id)).is_err()
                        || alias_count == MAX_JOINED_REBORROW_ALIASES
                    {
                        return None;
                    }
                    returned_aliases.insert(deref.id, (holder, rest.id));
                    holders.insert(alias, birth);
                    alias_count += 1;
                    continue;
                }
                if writable
                    && exact_named_scalar_store_pointer(deref, points, scalar_stores, grouped_installs)
                    == Some((rest.id, holder, None))
                {
                    continue;
                }
                if writable && birth != point.id && write_moves.binary_search_by_key(&deref.id, |&(moved, _, _)| moved).ok()
                    .is_some_and(|index| {
                        let (_, read, call) = write_moves[index];
                        read == rest.id && deref.successors.as_slice() == [call]
                            && matches!(&deref.kind, CfgPointKind::Move { owner, .. } if owner == holder)
                    })
                {
                    continue;
                }
                if !writable && matches!(&deref.kind, CfgPointKind::Call { output: 0, argument_points, .. }
                    if matches!(argument_points.as_slice(), [read] if *read == rest.id)
                        || matches!(argument_points.as_slice(), [scalar, read] if *scalar < *read && *read == rest.id))
                    && read_calls.binary_search(&(deref.id, rest.id)).is_ok()
                {
                    continue;
                }
                if !matches!(&deref.kind, CfgPointKind::ReadPlace {
                        place: ScalarPlace::Dereference { .. }, pointer_point: Some(ptr), reads_origin: false, ..
                    } if *ptr == rest.id)
                {
                    return None;
                }
                let checked = if deref.id.0 == rest.id.0 + 1 {
                    while next_pointee.first().is_some_and(|id| *id < deref.id) {
                        next_pointee = &next_pointee[1..];
                    }
                    next_pointee.first() == Some(&deref.id)
                } else {
                    pointee_reads.binary_search(&deref.id).is_ok()
                };
                if !checked {
                    return None;
                }
            }
            CfgPointKind::Assign { target, .. } | CfgPointKind::Borrow { place: target, .. }
            | CfgPointKind::Move { place: target, .. } | CfgPointKind::DeferredMove { place: target, .. }
                if (holders.contains_key(&target.binding)
                    && write_moves.binary_search_by_key(&rest.id, |&(moved, _, _)| moved).is_err())
                    || carrier_move.is_some_and(|moved| &target.binding == binding && moved != rest.id) =>
                return None,
            _ => (),
        }
    }
    (observed_root && (!nested || nested_exit.is_some())
        && descendants.keys().copied().eq(descendant_sites.iter().copied()))
        .then_some(JoinedReborrowShape { parent: binding, holders, descendants, returned_aliases, nested_exit })
}

struct JoinedLoanFamily<'a> {
    shape: JoinedReborrowShape<'a>,
    first_issuance: CfgPointId,
    selected_parents: Vec<ReferenceOriginId>,
}

impl JoinedLoanFamily<'_> {
    fn live_authorized_origin(
        &self, points: &[ReferenceFlowPoint], facts: &[ReferenceOrigin], state: &ReferenceEdgeState,
        holder: &ReferenceBindingId, read: CfgPointId,
    ) -> Option<ReferenceOriginId> {
        let mut issuance = *self.shape.holders.get(holder)?;
        let values = state.point_values.get(read)?;
        let origin = *values.first().filter(|_| values.len() == 1)?;
        if !exact_live_reborrow_holder(state, holder, origin) { return None; }
        let ReferenceOrigin::Fresh { loan, .. } = facts.get(origin.0)? else { return None };
        let mut current = loan;
        // Compare the current edge's exact loan chain with the authored
        // issuance chain; neither selected root's observation authorizes the other edge.
        for _ in 0..=MAX_JOINED_REBORROW_DESCENDANTS {
            let site = points.get(issuance.0)?;
            let CfgPointKind::ReborrowThrough { mode, .. } = site.kind else { return None };
            if current.mode != mode || current.creation_span != site.source_span.range {
                return None;
            }
            let parent = current.parent.as_deref()?;
            if issuance == self.first_issuance {
                let owner = &parent.origin_place;
                return (self.selected_parents.iter().any(|id| matches!(facts.get(id.0),
                    Some(ReferenceOrigin::Fresh { loan, .. }) if loan == parent))
                    && !state.released.contains(&owner.binding) && !state.invalidated.contains(&owner.binding)
                    && !state.uninitialized.contains(&owner.binding) && !unavailable_place(state, owner))
                    .then_some(origin);
            }
            issuance = self.shape.descendants.get(&issuance)?.0;
            current = parent;
        }
        None
    }
}

fn joined_direct_lexical_place(place: &ReferencePlaceId) -> bool {
    // Match the resolved loan path to the exact written place from leaf to
    // root. Do not infer a disjointness claim from an unresolved or dynamic
    // projection, even if a later component is a known field.
    let mut written = &place.place;
    for edge in place.projections.iter().rev() {
        written = match (written, edge) {
            (
                ScalarPlace::Field { base, field: ScalarFieldReference::Resolved(written), .. },
                ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(resolved)),
            ) if written == resolved => base.as_ref(),
            (
                ScalarPlace::Index { base, index, .. },
                ReferencePlaceProjection::Index(resolved @ ScalarExpression::Integer { .. }),
            ) if index.as_ref() == resolved => base.as_ref(),
            _ => return false,
        };
    }
    matches!(written, ScalarPlace::Name { .. })
}

fn joined_allocated_element_parent(site: &ReferenceFlowPoint, owner: &ReferencePlaceId) -> bool {
    // Unlike a lexical scalar, the storage has an independent cleanup
    // obligation. Only a direct reborrow of a distinct selected
    // allocation's already-typed constant scalar element enters this lane.
    // The family separately validates every mode, write and live descendant.
    matches!(&site.kind, CfgPointKind::ReborrowThrough {
        place: ScalarPlace::Dereference { .. }, ..
    }) && matches!(&owner.projections[..], [ReferencePlaceProjection::Index(
        ScalarExpression::Integer { value, .. }
    )] if value >= &0.into())
}

fn checked_joined_reborrow_parent(
    site: &ReferenceFlowPoint,
    facts: &[ReferenceOrigin],
    allocations: &BTreeSet<ReferenceBindingId>,
    pointer: CfgPointId,
    state: &ReferenceEdgeState,
    binding: &ReferenceBindingId,
) -> Option<ReferenceOriginId> {
    let origins = state.point_values.get(pointer)?;
    let origin = *origins.first().filter(|_| origins.len() == 1)?;
    let ReferenceOrigin::Fresh { loan, .. } = facts.get(origin.0)? else { return None };
    let owner = &loan.origin_place;
    if loan.mode != ScalarReferenceMutability::Mutable || loan.parent.is_some()
        || !joined_direct_lexical_place(owner)
        || owner.binding == *binding
        || (allocations.contains(&owner.binding) && !joined_allocated_element_parent(site, owner))
        || !contains_loan_site(&owner.binding, site)
        || !exact_live_reborrow_holder(state, binding, origin)
        || state.released.contains(&owner.binding) || state.invalidated.contains(&owner.binding)
        || state.uninitialized.contains(&owner.binding) || unavailable_place(state, owner)
    {
        return None;
    }
    Some(origin)
}

fn checked_joined_reborrow_parents(
    points: &[ReferenceFlowPoint],
    site: &ReferenceFlowPoint,
    facts: &[ReferenceOrigin],
    allocations: &BTreeSet<ReferenceBindingId>,
    pointer: CfgPointId,
    states: &[ReferenceEdgeState],
    binding: &ReferenceBindingId,
    requires_owned: bool,
    nested_exit: Option<&super::control::ScopeExitBindings>,
) -> Option<Vec<ReferenceOriginId>> {
    let minimum = if states.len() == 1 && matches!(&site.kind,
        CfgPointKind::ReborrowThrough { place: ScalarPlace::Index { base, .. }, .. }
            if direct_checked_field_base(base).is_some())
    { 1 } else { 2 };
    if !(minimum..=MAX_JOINED_REBORROW_PARENTS).contains(&states.len()) {
        return None;
    }
    let mut selected = Vec::with_capacity(states.len());
    let mut owners: Vec<&ReferencePlaceId> = Vec::with_capacity(states.len());
    for state in states {
        let id = checked_joined_reborrow_parent(site, facts, allocations, pointer, state, binding)?;
        // Unrelated branches may keep exactly the same issued loan. This is
        // not equivalent to separate overlapping issuances from the same place.
        if selected.contains(&id) {
            continue;
        }
        let ReferenceOrigin::Fresh { loan, .. } = &facts[id.0] else { return None };
        let owner = &loan.origin_place;
        let allocated = allocations.contains(&owner.binding);
        if (requires_owned || nested_exit.is_some()) && !allocated { return None; }
        if nested_exit.is_some_and(|exit| !exit.contains(&owner.binding)) { return None; }
        if owners.iter().any(|prior| {
            allocated != allocations.contains(&prior.binding)
                || (allocated && prior.binding == owner.binding)
                || reference_places_overlap(prior, owner)
        }) {
            return None;
        }
        selected.push(id);
        owners.push(owner);
    }
    if selected.len() < minimum {
        return None;
    }
    // Check exact same-scope lexical declarations with one shared prefix pass,
    // even if several disjoint alternatives belong to the same aggregate.
    let mut undeclared: BTreeSet<_> = owners.iter().map(|owner| &owner.binding).collect();
    for point in points.get(..pointer.0)? {
        if point.scope != site.scope {
            continue;
        }
        if let CfgPointKind::Bind { binding } = &point.kind {
            undeclared.remove(binding);
            if undeclared.is_empty() {
                return Some(selected);
            }
        }
    }
    None
}

fn joined_projection_field(place: &ScalarPlace) -> (&ScalarPlace, Option<&ScalarExpression>) {
    match place {
        ScalarPlace::Index { base, index, .. } => (base.as_ref(), Some(index.as_ref())),
        _ => (place, None),
    }
}

fn direct_checked_field_base(place: &ScalarPlace) -> Option<(&ScalarExpression, &ScalarFieldReference)> {
    let ScalarPlace::Field { base, field, .. } = place else { return None };
    let ScalarPlace::Dereference { pointer, .. } = base.as_ref() else { return None };
    Some((pointer, field))
}

#[derive(Clone, Copy)]
struct CheckedProjectedReborrowRead<'a> {
    pointer: CfgPointId,
    binding: &'a ReferenceBindingId,
    span: ByteSpan,
    origin: ReferenceOriginId,
}

fn checked_projected_reborrow_read<'a>(
    points: &'a [ReferenceFlowPoint],
    borrow: &ReferenceFlowPoint,
    authority: &super::control::NamedReborrowParent,
) -> Option<CheckedProjectedReborrowRead<'a>> {
    if borrow.id != authority.borrow {
        return None;
    }
    let read = points.get(authority.pointer.0)?;
    Some(CheckedProjectedReborrowRead {
        pointer: authority.pointer,
        binding: exact_reborrow_read_edge(read, borrow)?,
        span: read.source_span.range,
        origin: authority.origin,
    })
}

#[cfg(test)]
fn exact_projected_reborrow_parent_on_edge(
    state: &ReferenceEdgeState,
    points: &[ReferenceFlowPoint],
    facts: &[ReferenceOrigin],
    authority: &super::control::NamedReborrowParent,
    loan: &ReferenceLoanId,
) -> bool {
    points.get(authority.borrow.0)
        .and_then(|borrow| checked_projected_reborrow_read(points, borrow, authority))
        .is_some_and(|read| checked_projected_reborrow_parent_on_edge(state, facts, read, loan))
}

fn checked_projected_reborrow_parent_on_edge(
    state: &ReferenceEdgeState,
    facts: &[ReferenceOrigin],
    authority: CheckedProjectedReborrowRead<'_>,
    loan: &ReferenceLoanId,
) -> bool {
    let Some((ScalarExpression::Name { span, .. }, field @ ScalarFieldReference::Resolved(_))) =
        direct_checked_field_base(&loan.origin_place.place)
    else { return false };
    let [ReferencePlaceProjection::Dereference(projected_parent), ReferencePlaceProjection::Field(projected_field)] =
        loan.origin_place.projections.as_slice()
    else { return false };
    let Some(ReferenceOrigin::Fresh { loan: parent, .. }) = facts.get(authority.origin.0)
    else { return false };
    let binding = authority.binding;
    let owner = &parent.origin_place;
    if &authority.span != span
        || parent.parent.is_some() || !owner.projections.is_empty()
        || !matches!(owner.place, ScalarPlace::Name { .. })
        || owner.binding == *binding || owner.binding != loan.origin_place.binding
        || loan.parent.as_deref() != Some(parent) || projected_parent.as_ref() != parent
        || field != projected_field
        || (loan.mode == ScalarReferenceMutability::Mutable && parent.mode != ScalarReferenceMutability::Mutable)
    {
        return false;
    }
    state.point_values.get(authority.pointer)
        .is_some_and(|origins| origins.len() == 1 && origins.contains(&authority.origin))
        && exact_live_reborrow_holder(state, binding, authority.origin)
        && !state.released.contains(&owner.binding) && !state.invalidated.contains(&owner.binding)
        && !state.uninitialized.contains(&owner.binding) && !unavailable_place(state, owner)
}

fn exact_named_scalar_store_pointer<'a>(
    write: &'a ReferenceFlowPoint,
    points: &'a [ReferenceFlowPoint],
    typed_stores: &[CfgPointId],
    grouped_installs: &BTreeSet<CfgPointId>,
) -> Option<(CfgPointId, &'a ReferenceBindingId, Option<&'a ScalarStructFieldId>)> {
    let CfgPointKind::AssignThrough { place, pointer_point, rhs_point, .. } = &write.kind
    else { return None };
    let (pointer, field) = match place {
        ScalarPlace::Dereference { pointer, .. } => (pointer.as_ref(), None),
        place => match direct_checked_field_base(place)? {
            (pointer, ScalarFieldReference::Resolved(field)) => (pointer, Some(field)),
            _ => return None,
        },
    };
    let ScalarExpression::Name { span, .. } = pointer else { return None };
    if typed_stores.binary_search(&write.id).is_err()
        || grouped_installs.contains(&write.id)
        || !write.possible_origins.as_ref().is_some_and(BTreeSet::is_empty)
        || rhs_point >= pointer_point || pointer_point.0 + 1 != write.id.0
    {
        return None;
    }
    let read = points.get(pointer_point.0)?;
    let CfgPointKind::Read { binding } = &read.kind else { return None };
    (read.successors.as_slice() == [write.id] && read.scope == write.scope && &read.source_span.range == span)
        .then_some((*pointer_point, binding, field))
}

fn exact_named_indexed_store_pointer<'a>(
    write: &'a ReferenceFlowPoint,
    points: &'a [ReferenceFlowPoint],
    typed_stores: &[CfgPointId],
    grouped_installs: &BTreeSet<CfgPointId>,
) -> Option<(CfgPointId, &'a ReferenceBindingId, &'a ScalarStructFieldId, &'a ScalarExpression)> {
    let CfgPointKind::AssignThrough { place: ScalarPlace::Index { base, index, .. }, pointer_point, rhs_point, .. } =
        &write.kind else { return None };
    let (ScalarExpression::Name { span, .. }, ScalarFieldReference::Resolved(field)) = direct_checked_field_base(base)?
        else { return None };
    if !matches!(index.as_ref(), ScalarExpression::Integer { .. })
        || typed_stores.binary_search(&write.id).is_err() || grouped_installs.contains(&write.id)
        || !write.possible_origins.as_ref().is_some_and(BTreeSet::is_empty) || rhs_point >= pointer_point
    {
        return None;
    }
    let read = points.get(pointer_point.0)?;
    let CfgPointKind::Read { binding } = &read.kind else { return None };
    // The prepared store certificate matched this fixed array and index.
    // Keep the actual evaluated corridor (including a lexical Read or
    // literal arithmetic), and require it to end at the same source span as
    // the captured proof value. Never treat a call or write as index syntax.
    if read.scope != write.scope || &read.source_span.range != span { return None; }
    let index_span = expression_span(index);
    let mut cursor = read;
    for _ in 0..15 {
        let [next] = cursor.successors.as_slice() else { return None };
        let next = points.get(next.0)?;
        if cursor.id >= next.id || next.id >= write.id || next.scope != write.scope {
            return None;
        }
        let actual = next.source_span.range;
        if actual.start < index_span.start || actual.end > index_span.end
            || !matches!(&next.kind,
                CfgPointKind::Read { .. }
                | CfgPointKind::Evaluate(ScalarExpression::Integer { .. })
                | CfgPointKind::Evaluate(ScalarExpression::Binary {
                    operator: BinaryOperator::Add | BinaryOperator::Subtract | BinaryOperator::Multiply,
                    ..
                }))
        {
            return None;
        }
        if next.successors.as_slice() == [write.id] {
            return (actual == index_span).then_some((*pointer_point, binding, field, index.as_ref()));
        }
        cursor = next;
    }
    None
}

fn point_last_use_dependencies<'a>(
    kind: &'a CfgPointKind,
    pair: &'a mut [CfgPointId; 2],
) -> &'a [CfgPointId] {
    match kind {
        CfgPointKind::Invalidate { address_point, .. }
        | CfgPointKind::Release { target: ReleaseTarget::Checked { address_point, .. } } =>
            std::slice::from_ref(address_point),
        CfgPointKind::MoveThrough { pointer_point, .. }
        | CfgPointKind::DeferredMoveThrough { pointer_point, .. }
        | CfgPointKind::ReborrowThrough { pointer_point, .. } => std::slice::from_ref(pointer_point),
        CfgPointKind::Assign { rhs_point, .. } => std::slice::from_ref(rhs_point),
        CfgPointKind::ReturnOutput { value_point, .. } => std::slice::from_ref(value_point),
        CfgPointKind::ConditionalOutput { then_point, else_point } => {
            *pair = [*then_point, *else_point];
            pair
        }
        CfgPointKind::Call { argument_points, .. } => argument_points,
        CfgPointKind::AssignThrough { rhs_point, pointer_point, .. } => {
            *pair = [*rhs_point, *pointer_point];
            pair
        }
        CfgPointKind::ReadPlace { pointer_point, .. } => pointer_point.as_slice(),
        _ => &[],
    }
}

#[cfg(test)]
#[test]
fn point_last_use_dependencies_keep_canonical_order_and_storage() {
    let mut pair = [CfgPointId(0); 2];
    let conditional = CfgPointKind::ConditionalOutput {
        then_point: CfgPointId(2), else_point: CfgPointId(5),
    };
    assert_eq!(point_last_use_dependencies(&conditional, &mut pair), &[CfgPointId(2), CfgPointId(5)]);
    let returned = CfgPointKind::ReturnOutput { output: 0, value_point: CfgPointId(7), checked: true };
    let CfgPointKind::ReturnOutput { value_point, .. } = &returned else { unreachable!() };
    let dependencies = point_last_use_dependencies(&returned, &mut pair);
    assert_eq!(dependencies, &[CfgPointId(7)]);
    assert!(std::ptr::eq(&dependencies[0], value_point));
    assert!(point_last_use_dependencies(&CfgPointKind::ScopeExit, &mut pair).is_empty());
}

#[derive(PartialEq)]
enum ForwardedMove {
    Append,
    Unchanged,
    DuplicateUnchanged,
}

fn forwarded_move(
    moved: &[(ReferencePlaceId, SourceSpan)],
    place: &ReferencePlaceId,
    seen_unchanged: &mut bool,
    single_successor: bool,
) -> ForwardedMove {
    if !moved.iter().any(|(known, _)| known == place) {
        ForwardedMove::Append
    } else if single_successor && std::mem::replace(seen_unchanged, true) {
        ForwardedMove::DuplicateUnchanged
    } else {
        ForwardedMove::Unchanged
    }
}

fn forwarded_edge_seed(state: &mut Option<ReferenceEdgeState>, final_edge: bool) -> ReferenceEdgeState {
    if final_edge {
        state.take().expect("one seed for the final successor alternative")
    } else {
        state.as_ref().expect("seed for remaining successor alternatives").clone()
    }
}

#[cfg(test)]
#[test]
fn single_successor_move_skips_only_duplicate_unchanged_states() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let place = |start| {
        let declaration_span = ByteSpan::new(start, start + 1);
        ReferencePlaceId {
            source: source.clone(), function_span: span, block_span: span, declaration_span,
            binding: ReferenceBindingId {
                source: source.clone(), function_span: span, block_span: span, declaration_span,
                kind: ReferenceBindingKind::Declared,
            },
            projections: Vec::new(), place: ScalarPlace::Name { name: String::new(), span },
        }
    };
    let moved = [1, 2].map(|start| (place(start), SourceSpan::new(source.clone(), span)));
    let mut seen_unchanged = false;
    assert!(forwarded_move(&moved, &place(1), &mut seen_unchanged, true) == ForwardedMove::Unchanged);
    assert!(forwarded_move(&moved, &place(3), &mut seen_unchanged, true) == ForwardedMove::Append);
    assert!(forwarded_move(&moved, &place(2), &mut seen_unchanged, true) == ForwardedMove::DuplicateUnchanged);
    assert!(forwarded_move(&moved, &place(2), &mut seen_unchanged, false) == ForwardedMove::Unchanged);
}

#[cfg(test)]
#[test]
fn final_forwarded_alternative_reuses_the_unmodified_edge_seed() {
    let uses = [2, 2, 2];
    let id = CfgPointId(0);
    let original = Some(BTreeSet::from([ReferenceOriginId(0)]));
    let modified = Some(BTreeSet::from([ReferenceOriginId(1)]));
    let mut seed = Some(ReferenceEdgeState::default());
    seed.as_mut().unwrap().point_values.record(id, &original, &uses);
    let mut first = forwarded_edge_seed(&mut seed, false);
    first.point_values.record(id, &modified, &uses);
    let last = forwarded_edge_seed(&mut seed, true);
    assert!(seed.is_none());
    assert_eq!(first.point_values.get(id), modified.as_ref());
    assert_eq!(last.point_values.get(id), original.as_ref());
}

fn keep_forward_point_in_place(
    kind: &CfgPointKind,
    ordinary_forward: bool,
    incoming_states: usize,
) -> bool {
    ordinary_forward && incoming_states == 1
        && !matches!(kind, CfgPointKind::MoveThrough { .. } | CfgPointKind::DeferredMoveThrough { .. }
            | CfgPointKind::ReborrowThrough { .. })
}

fn grouped_assignment_installs(
    original_points: &[ReferenceFlowPoint],
    has_capture: Option<bool>,
) -> BTreeSet<CfgPointId> {
    if has_capture == Some(false) {
        return BTreeSet::new();
    }
    original_points.iter()
        .filter_map(|point| match &point.kind {
            CfgPointKind::CaptureAssignmentPrevious { writes } => Some(writes.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect()
}

#[cfg(test)]
#[test]
fn grouped_install_inventory_preserves_detached_and_canonical_capture() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let point = |kind| ReferenceFlowPoint {
        id: CfgPointId(0), kind, source_span: SourceSpan::new(source.clone(), span),
        scope: ReferenceScopeId(0), possible_origins: None, successors: Vec::new(),
    };
    let empty = [point(CfgPointKind::Entry)];
    assert!(grouped_assignment_installs(&empty, Some(false)).is_empty());
    let points = [point(CfgPointKind::CaptureAssignmentPrevious {
        writes: vec![CfgPointId(1), CfgPointId(2)],
    })];
    let expected = BTreeSet::from([CfgPointId(1), CfgPointId(2)]);
    assert_eq!(grouped_assignment_installs(&points, Some(true)), expected);
    assert_eq!(grouped_assignment_installs(&points, None), expected);
}

enum ReferenceStateBatch {
    Single(Option<ReferenceEdgeState>),
    Many(std::vec::IntoIter<ReferenceEdgeState>),
}

impl ReferenceStateBatch {
    fn as_slice(&self) -> &[ReferenceEdgeState] {
        match self {
            Self::Single(state) => state.as_slice(),
            Self::Many(states) => states.as_slice(),
        }
    }
}

impl Iterator for ReferenceStateBatch {
    type Item = ReferenceEdgeState;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Single(state) => state.take(),
            Self::Many(states) => states.next(),
        }
    }
}

enum ReferenceIncomingStates {
    Linear { destination: CfgPointId, states: Vec<ReferenceEdgeState> },
    Indexed(Vec<Vec<ReferenceEdgeState>>),
}

impl ReferenceIncomingStates {
    fn new(point_count: usize, linear: bool) -> Self {
        if linear {
            Self::Linear { destination: CfgPointId(0), states: Vec::new() }
        } else {
            Self::Indexed(vec![Vec::new(); point_count])
        }
    }

    fn next_forward(&self, cursor: usize) -> Option<CfgPointId> {
        match self {
            Self::Linear { destination, states } => {
                debug_assert!(states.is_empty() || destination.0 >= cursor);
                (!states.is_empty()).then_some(*destination)
            }
            Self::Indexed(slots) => slots[cursor..].iter().position(|slot| !slot.is_empty())
                .map(|offset| CfgPointId(cursor + offset)),
        }
    }

    fn take(&mut self, id: CfgPointId) -> ReferenceStateBatch {
        let linear = matches!(self, Self::Linear { .. });
        let slot = self.slot(id);
        if linear && slot.len() == 1 {
            ReferenceStateBatch::Single(slot.pop())
        } else {
            ReferenceStateBatch::Many(std::mem::take(slot).into_iter())
        }
    }

    fn slot(&mut self, id: CfgPointId) -> &mut Vec<ReferenceEdgeState> {
        match self {
            Self::Linear { destination, states } => {
                debug_assert!(states.is_empty() || *destination == id);
                *destination = id;
                states
            }
            Self::Indexed(slots) => &mut slots[id.0],
        }
    }
}

#[cfg(test)]
#[test]
fn linear_incoming_slot_preserves_gaps_and_distinct_ordered_states() {
    let mut incoming = ReferenceIncomingStates::new(8, true);
    incoming.slot(CfgPointId(0)).push(ReferenceEdgeState::default());
    assert_eq!(incoming.next_forward(0), Some(CfgPointId(0)));
    let allocation = incoming.slot(CfgPointId(0)).as_ptr();
    let capacity = incoming.slot(CfgPointId(0)).capacity();
    let mut initial = incoming.take(CfgPointId(0));
    assert_eq!(initial.as_slice().len(), 1);
    let mut first = initial.next().unwrap();
    assert_eq!(incoming.next_forward(1), None);
    assert_eq!(incoming.slot(CfgPointId(4)).as_ptr(), allocation);
    assert_eq!(incoming.slot(CfgPointId(4)).capacity(), capacity);

    first.value = Some(BTreeSet::from([ReferenceOriginId(0)]));
    let mut second = ReferenceEdgeState::default();
    second.value = Some(BTreeSet::from([ReferenceOriginId(1)]));
    incoming.slot(CfgPointId(4)).push(first.clone());
    assert_eq!(incoming.slot(CfgPointId(4)).as_ptr(), allocation);
    incoming.slot(CfgPointId(4)).push(second.clone());
    assert_eq!(incoming.next_forward(1), Some(CfgPointId(4)));
    let states = incoming.take(CfgPointId(4));
    assert!(states.as_slice() == &[first, second]);
    assert_eq!(incoming.slot(CfgPointId(4)).capacity(), 0);
    assert_eq!(incoming.next_forward(5), None);
    incoming.slot(CfgPointId(7)).push(ReferenceEdgeState::default());
    assert_eq!(incoming.next_forward(5), Some(CfgPointId(7)));
}

fn require_available_fallible_read(
    bindings: &HashMap<ReferenceBindingId, super::control::GuardedAllocationBinding>,
    state: &ReferenceEdgeState,
    binding: &ReferenceBindingId,
    point: &ReferenceFlowPoint,
    whole: bool,
    errors: &mut Vec<ReferenceAnalysisError>,
) {
    let unavailable = state.uninitialized.contains(binding) || state.released.contains(binding)
        || state.invalidated.contains(binding);
    let Some(site) = (whole || unavailable).then(|| bindings.get(binding)).flatten() else { return };
    let reason = if whole && site.has_indexed_extent() {
        ReferenceAnalysisErrorReason::UnsupportedFallibleAllocationAccess
    } else if whole && matches!(site.ty, ScalarType::Struct(_)) && !site.whole_copy {
        ReferenceAnalysisErrorReason::UnsupportedFallibleWholeAggregateRead
    } else if unavailable {
        ReferenceAnalysisErrorReason::UnavailableFallibleAllocation
    } else {
        return;
    };
    errors.push(ReferenceAnalysisError::MissingOrigin {
        source_span: point.source_span.clone(),
        binding_id: binding.clone(),
        reason,
    });
}

fn guarded_allocation_overwrite_error(
    whole_copy: bool, whole_direct: bool, unavailable: bool,
) -> Option<ReferenceAnalysisErrorReason> {
    if !whole_copy || !whole_direct {
        Some(ReferenceAnalysisErrorReason::UnsupportedFallibleAllocationAccess)
    } else if unavailable {
        Some(ReferenceAnalysisErrorReason::UnavailableFallibleAllocation)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallOriginTransfer<'a> {
    Explicit(&'a [ReferenceOrigin]),
    WholeFormal(usize),
}

fn transfer_reference_facts(
    mut cfg: ReferenceTransferCfg<'_>,
    deferred_policies: &BTreeMap<usize, CopyPolicy>,
    contracts: &HashMap<CfgPointId, CallOriginTransfer<'_>>,
    mut aggregate: Option<(
        &aggregate::AggregateFlowFacts,
        aggregate::AggregateObservationMode,
        &FunctionCallSites,
        &mut aggregate::AggregateFlowResult,
    )>,
    mut descriptor: Option<&mut ScalarDescriptorReplay<'_>>,
) -> Vec<ReferenceAnalysisError> {
    if !deferred_policies.is_empty() || !contracts.is_empty() || cfg.pending_named_reborrow.is_some() {
        cfg.local_borrow = None;
    }
    let original_points = cfg.initial_points;
    let grouped_installs = grouped_assignment_installs(original_points, cfg.has_grouped_assignment_capture);
    let mut last_use = (0..cfg.points.len()).collect::<Vec<_>>();
    let mut strictly_forward = true;
    let mut unbranched = !matches!(cfg.forward_initial_graph, Some(super::control::ForwardInitialGraph::Branched));
    let validate_topology = cfg.forward_initial_graph.is_none();
    let indexed_holders = cfg.indexed_holders.filter(|_| cfg.raw_address_exposures.is_empty() && deferred_policies.is_empty());
    let mut disabled_indexed_bindings = HashSet::new();
    let mut projected_leaf_sites = HashMap::new();
    let mut projected_leaf_holders = HashMap::new();
    let mut projected_store_facts = None::<(BTreeSet<ReferenceOriginId>, HashMap<CfgPointId, ReferenceOriginId>)>;
    let mut projected_exposures = None;
    let mut known_lexical_owners = None::<HashMap<&ReferenceBindingId, Option<ReferencePlaceId>>>;
    let mut aggregate_expiry_allowed = aggregate.is_some();
    let pending = cfg.pending_named_reborrow;
    let mut local_definition = None;
    let mut child_holders = HashMap::new();
    let mut pending_allowed = pending.is_some_and(|pending| {
        let has_shared = pending.sites.iter().any(|site|
            is_pending_shared_reborrow(&original_points[site.point.0]));
        if pending.sites.len() > 1 || has_shared || pending.sites.first().is_some_and(|site|
            deferred_local_reborrow_differs_in_scope(pending, &original_points[site.point.0], original_points))
        {
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
        if child_holders.is_empty() {
            return None;
        }
        let (_, binding, None) = exact_named_scalar_store_pointer(
            write, original_points, cfg.checked_named_scalar_stores, &grouped_installs,
        )? else { return None };
        child_holders.get(binding).copied()
    };
    let mut dependency_pair = [CfgPointId(0); 2];
    let mut local_fork_index = 0;
    for (index, point) in original_points.iter().enumerate() {
        if let Some(binding) = indexed_holders.and_then(|holders| holders.disabled_by(point)) {
            disabled_indexed_bindings.insert(binding);
        }
        if cfg.local_borrow.is_some_and(|sealed| !sealed.matches_edge_shape(index, point, &mut local_fork_index)) {
            cfg.local_borrow = None;
        }
        if validate_topology {
            strictly_forward = strictly_forward && point.id.0 == index
                && point.successors.iter().all(|successor| successor.0 > index && successor.0 < original_points.len());
            unbranched = unbranched && point.successors.len() <= 1;
        }
        aggregate_expiry_allowed = aggregate_expiry_allowed && strictly_forward
            && !matches!(&point.kind, CfgPointKind::MoveThrough { place, .. }
                if !matches!(place, ScalarPlace::Dereference { .. }));
        match &point.kind {
            CfgPointKind::Borrow { place, loan } if loan.parent.is_none()
                && (!place.projections.is_empty() || (
                    matches!(place.place, ScalarPlace::Name { .. })
                        && matches!(loan.origin_place.place, ScalarPlace::Name { .. })
                        && loan.origin_place.projections.is_empty()
                        && loan.origin_place.binding == place.binding
                )) =>
            {
                if let Some(root) = direct_lexical_loan_root(place) {
                    let owner = known_lexical_owners.get_or_insert_with(HashMap::new).entry(&place.binding).or_insert(None);
                    if owner.is_none() && loan.mode == ScalarReferenceMutability::Mutable {
                        *owner = Some(ReferencePlaceId {
                            source: place.source.clone(),
                            function_span: place.function_span,
                            declaration_span: place.declaration_span,
                            block_span: place.block_span,
                            binding: place.binding.clone(),
                            projections: Vec::new(),
                            place: root.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
        match &point.kind {
            CfgPointKind::Borrow { loan, .. }
                if direct_checked_field_base(&loan.origin_place.place).is_some() =>
            {
                let authority = cfg.named_reborrow_parents
                    .binary_search_by_key(&point.id, |parent| parent.borrow)
                    .ok().map(|index| &cfg.named_reborrow_parents[index]);
                if let (Some(authority), [next]) = (authority, point.successors.as_slice()) {
                    let parent = checked_projected_reborrow_read(original_points, point, authority);
                    let holder = original_points.get(next.0);
                    if let (Some(parent),
                        Some(ReferenceFlowPoint { kind: CfgPointKind::Bind { binding }, scope, .. }))
                        = (parent, holder)
                    {
                        let owner = &loan.origin_place.binding;
                        let (unknown, exposed) = projected_exposures.get_or_insert_with(|| {
                            (cfg.raw_address_exposures.iter().any(|(_, root)| root.is_none()),
                             cfg.raw_address_exposures.iter().filter_map(|(_, root)| root.as_ref())
                                .collect::<HashSet<_>>())
                        });
                        let exact_init = cfg.checked_binding_initializers
                            .binary_search_by_key(next, |(bind, _)| *bind)
                            .is_ok_and(|index| cfg.checked_binding_initializers[index].1 == point.id);
                        if point.id < *next && point.scope == *scope && exact_init && owner != binding
                            && parent.binding != binding && !cfg.allocations.contains(binding)
                            && !*unknown && !exposed.contains(owner) && !exposed.contains(parent.binding)
                            && !exposed.contains(binding)
                            && !projected_leaf_holders.contains_key(binding)
                        {
                            let admitted_origin = (!cfg.checked_named_scalar_stores.is_empty()
                                && loan.parent.as_deref().is_some_and(|parent| parent.mode == ScalarReferenceMutability::Mutable))
                                .then(|| point.possible_origins.as_ref()
                                    .filter(|set| set.len() == 1).and_then(BTreeSet::first).copied())
                                .flatten();
                            if let Some(origin) = admitted_origin {
                                projected_store_facts.get_or_insert_with(Default::default).0.insert(origin);
                            }
                            projected_leaf_holders.insert(binding, (point.id, point.scope, loan.mode, admitted_origin));
                            projected_leaf_sites.insert(point.id, parent);
                        }
                    }
                }
            }
            CfgPointKind::Read { binding } => {
                if let Some(&(borrow, scope, mode, admitted_origin)) = projected_leaf_holders.get(binding) {
                    let authorized = match point.successors.as_slice() {
                        [next] => original_points.get(next.0).is_some_and(|next| {
                            if point.scope != next.scope {
                                return false;
                            }
                            match &next.kind {
                                CfgPointKind::ReadPlace {
                                    place: ScalarPlace::Dereference { pointer, .. },
                                    pointer_point: Some(pointer_id), reads_origin: false, ..
                                } => (point.scope == scope || scope == ReferenceScopeId(0))
                                    && *pointer_id == point.id && matches!(pointer.as_ref(), ScalarExpression::Name { .. })
                                    && cfg.checked_pointee_reads.binary_search(&next.id).is_ok(),
                                CfgPointKind::AssignThrough { .. }
                                    if mode == ScalarReferenceMutability::Mutable
                                        && (point.scope == scope || (scope == ReferenceScopeId(0) && admitted_origin.is_some())) => {
                                    let exact = exact_named_scalar_store_pointer(
                                        next, original_points, cfg.checked_named_scalar_stores, &grouped_installs,
                                    ) == Some((point.id, binding, None));
                                    if exact {
                                        if let (Some(origin), Some((_, stores))) =
                                            (admitted_origin, &mut projected_store_facts)
                                        {
                                            stores.insert(next.id, origin);
                                        }
                                    }
                                    exact
                                },
                                _ => false,
                            }
                        }),
                        _ => false,
                    };
                    if !authorized {
                        projected_leaf_sites.remove(&borrow);
                        if let (Some(origin), Some((admitted, _))) = (admitted_origin, &mut projected_store_facts) {
                            admitted.remove(&origin);
                        }
                    }
                }
            }
            CfgPointKind::Assign { target, .. } | CfgPointKind::Move { place: target, .. }
            | CfgPointKind::DeferredMove { place: target, .. } | CfgPointKind::Borrow { place: target, .. } => {
                if let Some(&(borrow, _, _, admitted_origin)) = projected_leaf_holders.get(&target.binding) {
                    projected_leaf_sites.remove(&borrow);
                    if let (Some(origin), Some((admitted, _))) = (admitted_origin, &mut projected_store_facts) {
                        admitted.remove(&origin);
                    }
                }
            }
            _ => {}
        }
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
        let dependencies = point_last_use_dependencies(&point.kind, &mut dependency_pair);
        for dependency in dependencies {
            last_use[dependency.0] = point.id.0;
        }
    }
    if let Some(out) = cfg.forward_graph_out.as_deref_mut() {
        *out = strictly_forward.then_some(if unbranched {
            super::control::ForwardInitialGraph::Linear
        } else {
            super::control::ForwardInitialGraph::Branched
        });
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
    let indexed_holder = |binding: &ReferenceBindingId| {
        let holder = indexed_holders?.by_holder.get(binding)?;
        (!disabled_indexed_bindings.contains(binding)
            && !disabled_indexed_bindings.contains(&holder.selector)).then_some(holder)
    };
    let aggregate_deadlines = aggregate.as_ref()
        .filter(|_| aggregate_expiry_allowed)
        .map(|(facts, _, _, _)| facts.value_deadlines(&last_use));
    let point_count = cfg.points.len();
    let mut observed = None;
    let mut incoming = ReferenceIncomingStates::new(point_count, strictly_forward && unbranched);
    incoming.slot(CfgPointId(0)).push(ReferenceEdgeState {
        aggregate: aggregate.as_ref().map(|(facts, _, _, _)| facts.entry_state()),
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
    let liveness = ConflictLiveness::new(original_points, cfg.scope_exit_bindings, strictly_forward, !unbranched);
    let mut forward_cursor = 0;
    let mut joined_family = None;
    let mut forward_single_observation = None;
    let ordinary_forward = strictly_forward && pending.is_none() && descriptor.is_none();
    let mut errors = Vec::new();
    let mut pending_sites = pending.map_or(&[][..], |pending| pending.sites.as_slice()).iter().peekable();
    // Both indexes are derived once from the exact prepared whole-move
    // receipts. Only enrolled owners use the edge-local specialized lane.
    let (guarded_move_points, guarded_move_owners) = cfg.guarded_runtime_moves.map_or_else(
        || (HashMap::new(), HashSet::new()),
        |moves| moves.values().map(|receipt| ((receipt.moved, &receipt.source), &receipt.source)).unzip(),
    );
    loop {
        let id = if let Some((_, queued, ready)) = &mut fallback {
            let Some(id) = ready.pop_front() else { break };
            queued[id.0] = false;
            id
        } else {
            let Some(id) = incoming.next_forward(forward_cursor) else { break };
            forward_cursor = id.0 + 1;
            id
        };
        let common_aggregate_cutoff = aggregate_deadlines.as_ref()
            .and_then(|_| original_points[id.0].successors.iter().copied().min());
        let guarded_allocation_edge = cfg.guarded_allocation_edges
            .and_then(|edges| edges.get(&id))
            .filter(|edge| matches!(original_points[id.0].kind, CfgPointKind::Branch { .. })
                    && edge.present != edge.absent
                    && matches!(original_points[id.0].successors.as_slice(), [first, second]
                        if (*first, *second) == (edge.present, edge.absent)
                            || (*first, *second) == (edge.absent, edge.present)));
        let enrolled_move_owner = guarded_allocation_edge.is_some_and(|edge| guarded_move_owners.contains(&edge.binding));
        let states = incoming.take(id);
        let incoming_count = states.as_slice().len();
        let mut local_access_conflicts = None;
        let keep_in_place = keep_forward_point_in_place(&original_points[id.0].kind, ordinary_forward, incoming_count);
        let mut snapshot = if keep_in_place {
            None
        } else {
            Some(&mut observed.get_or_insert_with(|| vec![None::<ReferenceFlowPoint>; point_count])[id.0])
        };
        if strictly_forward && incoming_count == 1
            && matches!(&original_points[id.0].kind, CfgPointKind::MoveThrough { place, .. }
                if !matches!(place, ScalarPlace::Dereference { .. }))
        {
            forward_single_observation.get_or_insert_with(|| vec![false; cfg.points.len()])[id.0] = true;
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
            let read = original_points.get(site.read.0)?;
            let borrow = &original_points[id.0];
            let Some((producer, holder)) = site.parent else {
                return exact_pending_reborrow_read(read, borrow, &pending.handle)
                    .then_some((pending.origin, &pending.handle));
            };
            let declaration = &original_points[holder.0];
            let CfgPointKind::Bind { binding } = &declaration.kind else { return None };
            if producer >= holder || holder >= site.read
                || !exact_pending_reborrow_read(read, borrow, binding)
                || !contains_loan_site(binding, borrow)
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
                            && (site.parent.is_some() || initializer.scope == point.scope
                                || contains_loan_site(&pending.handle, &point))
                            && matches!(&initializer.kind, CfgPointKind::Borrow { place, loan }
                                if place.binding == *owner && loan.parent.is_none())
                            && initializer.possible_origins.as_ref()
                                .is_some_and(|origins| origins.len() == 1 && origins.contains(&pending.origin))
                    }
                }
                && incoming_count != 0
                && selected_parent.is_some_and(|(origin, binding)| states.as_slice().iter().all(|state| {
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
                    let child_id = if let Some(index) = cfg.provisional_origin_index.as_deref_mut() {
                        index.intern(cfg.facts, child)
                    } else if let Some(index) = cfg.facts.iter().position(|origin| *origin == child) {
                        ReferenceOriginId(index)
                    } else {
                        let id = ReferenceOriginId(cfg.facts.len());
                        cfg.facts.push(child);
                        id
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
        let named_parent = || cfg.named_reborrow_parents
            .binary_search_by_key(&id, |parent| parent.borrow)
            .ok().map(|index| &cfg.named_reborrow_parents[index]);
        let joined_cardinality = (2..=MAX_JOINED_REBORROW_PARENTS).contains(&incoming_count)
            || (incoming_count == 1 && matches!(&initial_point.kind,
                CfgPointKind::ReborrowThrough { place: ScalarPlace::Index { base, .. }, .. }
                    if direct_checked_field_base(base).is_some()));
        let joined_reborrow_binding = if strictly_forward && joined_cardinality
            && cfg.joined_reborrow_descendants.binary_search(&id).is_err()
            && matches!(initial_point.kind, CfgPointKind::ReborrowThrough { .. })
        {
            joined_root_reborrow_binding(
                original_points, initial_point, cfg.checked_binding_initializers, cfg.checked_pointee_reads,
                cfg.joined_shared_aliases, cfg.joined_reborrow_descendants, cfg.shared_scalar_read_calls,
                cfg.exclusive_scalar_write_moves, cfg.joined_shared_return_calls, cfg.joined_checked_releases, cfg.scope_exit_bindings,
                cfg.checked_named_scalar_stores, &grouped_installs, cfg.raw_address_exposures,
            )
        } else {
            None
        };
        let joined_reborrow_parents = match (&initial_point.kind, joined_reborrow_binding.as_ref()) {
            (CfgPointKind::ReborrowThrough { pointer_point, .. }, Some(shape)) =>
                checked_joined_reborrow_parents(
                    original_points, initial_point, cfg.facts, cfg.allocations, *pointer_point,
                    states.as_slice(), shape.parent, !cfg.joined_checked_releases.is_empty(), shape.nested_exit,
                ),
            _ => None,
        };
        if let (Some(shape), Some(parents)) = (joined_reborrow_binding, joined_reborrow_parents.as_ref()) {
            if !shape.descendants.is_empty()
                || matches!(&initial_point.kind, CfgPointKind::ReborrowThrough { mode, place, .. }
                    if *mode == ScalarReferenceMutability::Shared
                        || direct_checked_field_base(joined_projection_field(place).0).is_some())
            {
                joined_family = Some(JoinedLoanFamily { shape, first_issuance: id, selected_parents: parents.clone() });
            }
        }
        let joined_call = joined_family.as_ref().and_then(|family| {
            if !matches!(initial_point.kind, CfgPointKind::Call { output: 0, .. }) { return None; }
            let index = cfg.shared_scalar_read_calls.binary_search_by_key(&id, |&(call, _)| call).ok()?;
            let argument = cfg.shared_scalar_read_calls[index].1;
            let CfgPointKind::Read { binding } = &original_points[argument.0].kind else { return None };
            let issuance = *family.shape.holders.get(binding)?;
            matches!(original_points[issuance.0].kind, CfgPointKind::ReborrowThrough {
                mode: ScalarReferenceMutability::Shared, ..
            }).then_some((family, binding, argument))
        });
        let joined_write = joined_family.as_ref().and_then(|family| {
            if !matches!(initial_point.kind, CfgPointKind::Move { .. }) { return None; }
            let index = cfg.exclusive_scalar_write_moves.binary_search_by_key(&id, |&(moved, _, _)| moved).ok()?;
            let (_, argument, _) = cfg.exclusive_scalar_write_moves[index];
            let CfgPointKind::Read { binding } = &original_points[argument.0].kind else { return None };
            let issuance = *family.shape.holders.get(binding)?;
            (issuance != family.first_issuance && matches!(original_points[issuance.0].kind, CfgPointKind::ReborrowThrough {
                mode: ScalarReferenceMutability::Mutable, ..
            })).then_some((family, binding, argument))
        });
        let joined_return = joined_family.as_ref().and_then(|family| {
            let &(holder, argument) = family.shape.returned_aliases.get(&id)?;
            Some((family, holder, argument))
        });
        let projected_loan = match &initial_point.kind {
            CfgPointKind::Borrow { place, loan } if direct_checked_field_base(&place.place).is_some() => Some(loan),
            _ => None,
        };
        let projected_authority = projected_loan.and_then(|_| projected_leaf_sites.get(&id).copied());
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
            let projected_rejected = projected_loan.is_some_and(|loan| {
                !projected_authority.is_some_and(|authority| {
                    checked_projected_reborrow_parent_on_edge(&state, cfg.facts, authority, loan)
                })
            });
            if !projected_rejected {
                if let (Some((facts, mode, sites, result)), Some(flow)) =
                    (&mut aggregate, &mut state.aggregate)
                {
                    facts.visit(*mode, initial_point, original_points, flow, result, sites, aggregate_deadlines.as_deref());
                }
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
            if let Some((family, holder, argument)) = joined_call {
                let valid = family.live_authorized_origin(original_points, cfg.facts, &state, holder, argument)
                    .and_then(|origin| match cfg.facts.get(origin.0)? {
                        ReferenceOrigin::Fresh { loan, .. } => Some(loan),
                        _ => None,
                    });
                if let Some(loan) = valid {
                    let origins = conflicting_live_loan(
                        &liveness, &cfg.facts,
                        &state, id, &loan.origin_place, LoanAccess::ReadThrough(loan),
                    );
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: initial_point.source_span.clone(), origins,
                        });
                    }
                } else {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: initial_point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    });
                }
            }
            if let Some((family, holder, argument)) = joined_write {
                let candidate = family.live_authorized_origin(original_points, cfg.facts, &state, holder, argument)
                    .and_then(|origin| match cfg.facts.get(origin.0)? {
                        ReferenceOrigin::Fresh { loan, .. } if loan.mode == ScalarReferenceMutability::Mutable =>
                            Some(InvalidationCandidate { origin, place: loan.origin_place.clone(), loan: loan.clone() }),
                        _ => None,
                    });
                if let Some(candidate) = candidate {
                    let place = available_place(&candidate.place);
                    let origins = conflicting_live_loan(&liveness, &cfg.facts, &state, id, &place,
                        LoanAccess::WriteThrough(&candidate, None));
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: initial_point.source_span.clone(), origins,
                        });
                    }
                } else {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: initial_point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                    });
                }
            }
            let call_origins = if let Some((family, holder, argument)) = joined_return {
                let origin = family.live_authorized_origin(original_points, cfg.facts, &state, holder, argument);
                if origin.is_none() {
                    errors.push(ReferenceAnalysisError::MissingContext {
                        source_span: initial_point.source_span.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                    });
                }
                Some(origin.into_iter().collect())
            } else if let CfgPointKind::Call { output, argument_points, .. } =
                &cfg.points[id.0].kind
            {
                contracts.get(&id).map(|&transfer| {
                    let origins = match transfer {
                        CallOriginTransfer::Explicit(origins) => origins,
                        CallOriginTransfer::WholeFormal(parameter) => {
                            return argument_points.get(parameter).and_then(|argument| state.point_values.get(*argument))
                                .filter(|origins| !origins.is_empty()).cloned().unwrap_or_else(|| {
                                    errors.push(ReferenceAnalysisError::MissingContext {
                                        source_span: cfg.points[id.0].source_span.clone(),
                                        reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                    });
                                    BTreeSet::new()
                                });
                        }
                    };
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
                                            &liveness, &cfg.facts,
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
            if let CfgPointKind::Invalidate { operation, candidates, .. } = &point.kind {
                for candidate in candidates {
                    let origins = conflicting_live_loan(
                        &liveness, &cfg.facts,
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
                for binding in candidates.iter().map(|candidate| &candidate.place.binding) {
                    if cfg.array_length_owners.is_some_and(|owners| owners.contains(binding)) {
                        state.replaced_array_extents.insert(binding.clone());
                    }
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
                        && (candidates.is_empty() || !known.refers_to_owner(&cfg.facts, |binding| {
                            candidates.iter().any(|candidate| &candidate.place.binding == binding)
                        }))
                });
            }
            if let CfgPointKind::Release {
                target: ReleaseTarget::Checked { address_span, candidates, .. },
            } = &point.kind {
                // A checked address can select the allocation through an
                // element. LLVM frees the recorded owner backing, not the
                // element address, so every loan of that owner must end here.
                let owners: BTreeMap<_, _> = candidates.iter().map(|candidate|
                    (&candidate.place.binding, &candidate.place)).collect();
                for (&binding, &selection) in &owners {
                    let guarded = cfg.guarded_releases.and_then(|releases| releases.get(&id)) == Some(binding)
                        && owners.len() == 1 && selection.projections.is_empty();
                    if guarded {
                        if let Some(bindings) = cfg.fallible_allocation_bindings {
                            require_available_fallible_read(bindings, &state, binding, point, false, &mut errors);
                        }
                    }
                    if !guarded && !cfg.allocations.contains(binding) {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: address_span.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                        });
                        continue;
                    }
                    let mut owner = selection.clone();
                    owner.projections.clear();
                    let origins = conflicting_live_loan(
                        &liveness, &cfg.facts, &state, id, &owner, LoanAccess::Direct,
                    );
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(), origins,
                        });
                    }
                    if !state.released.insert(binding.clone()) {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: point.source_span.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                        });
                    }
                }
                if !owners.is_empty() {
                    retire_allocation_loans(&mut state, &cfg.facts, |binding| owners.contains_key(binding));
                }
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
                retire_allocation_loans(&mut state, &cfg.facts, |owner| owner == binding);
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
                            previous_candidate_origins(&state, &cfg.facts, &candidates, None)
                        }
                        _ => unreachable!("captured operation is an assignment install"),
                    };
                    state.assignment_previous.insert(*write, origins);
                }
            }
            let definite_assignment = match &point.kind {
                CfgPointKind::AssignThrough { candidates, pointer_point, .. } =>
                    definite_assignment_candidate(candidates, state.point_values.get(*pointer_point)).is_some(),
                _ => false,
            };
            let projected_write = (|| {
                let CfgPointKind::AssignThrough { candidates, .. } = &point.kind else { return None };
                let [candidate] = candidates.as_slice() else { return None };
                if candidate.loan.mode != ScalarReferenceMutability::Mutable {
                    return None;
                }
                if let Some((admitted, _)) = projected_store_facts.as_ref().filter(|(admitted, stores)|
                    stores.get(&id) == Some(&candidate.origin) && admitted.contains(&candidate.origin))
                {
                    let (ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. }) =
                        cfg.facts.get(candidate.origin.0)? else { return None };
                    let (parent, field) = exact_projected_field_of_mutable_parent(loan)?;
                    let place = available_place(&candidate.place);
                    return (place.binding == parent.origin_place.binding
                        && matches!(place.projections.as_slice(),
                            [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(actual))] if actual == field))
                        .then_some(DisjointProjectedWrite {
                            admission: ProjectedWriteAdmission::Known(admitted), parent, field,
                        });
                }
                let (read, binding, field, index) = if let Some((read, binding, Some(field))) = exact_named_scalar_store_pointer(
                    initial_point, original_points, cfg.checked_named_scalar_stores, &grouped_installs,
                ) {
                    (read, binding, field, None)
                } else {
                    let (read, binding, field, index) = exact_named_indexed_store_pointer(
                        initial_point, original_points, cfg.checked_named_scalar_stores, &grouped_installs,
                    )?;
                    (read, binding, field, Some(index))
                };
                let ReferenceOrigin::Fresh { loan: parent, .. } = cfg.facts.get(candidate.origin.0)?
                else { return None };
                let joined = joined_family.as_ref().and_then(|family| {
                    let issuance = original_points.get(family.first_issuance.0)?;
                    let CfgPointKind::ReborrowThrough { mode, place, .. } = &issuance.kind else { return None };
                    if family.shape.parent != binding || !family.shape.descendants.is_empty()
                        || !family.selected_parents.contains(&candidate.origin)
                    {
                        return None;
                    }
                    let (child_field, child_index) = joined_projection_field(place);
                    if direct_checked_field_base(child_field).is_none() || child_index.is_some() != index.is_some() {
                        return None;
                    }
                    Some(ProjectedWriteAdmission::Joined { issuance: issuance.source_span.range, mode: *mode, index })
                });
                let admission = if index.is_some() { joined? } else {
                    joined.or_else(|| projected_store_facts.as_ref()
                        .map(|(admitted, _)| ProjectedWriteAdmission::Known(admitted)))?
                };
                let owner = &parent.origin_place;
                (parent == &candidate.loan && parent.parent.is_none() && owner.projections.is_empty()
                    && matches!(owner.place, ScalarPlace::Name { .. })
                    && owner.binding != *binding && candidate.place.binding == owner.binding
                    && match (index, candidate.place.projections.as_slice()) {
                        (None, [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(actual))]) => actual == field,
                        (Some(index), [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(actual)),
                            ReferencePlaceProjection::Index(written)]) => actual == field && index == written,
                        _ => false,
                    }
                    && state.point_values.get(read).is_some_and(|origins|
                        origins.len() == 1 && origins.contains(&candidate.origin))
                    && exact_live_reborrow_holder(&state, binding, candidate.origin)
                    && !state.released.contains(&owner.binding) && !state.invalidated.contains(&owner.binding)
                    && !state.uninitialized.contains(&owner.binding) && !unavailable_place(&state, owner))
                    .then_some(DisjointProjectedWrite { admission, parent, field })
            })();
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
                        previous_candidate_origins(&state, &cfg.facts, candidates, projected_write)
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
            if let Some(owners) = &known_lexical_owners {
                let (whole, projected) = match &point.kind {
                    CfgPointKind::Read { binding } => (owners.get(binding).and_then(Option::as_ref), &[][..]),
                    CfgPointKind::ReadPlace { candidates, pointer_point: None, .. } => (None, candidates.as_slice()),
                    _ => (None, &[][..]),
                };
                for place in whole.into_iter().chain(projected) {
                    if !owners.get(&place.binding).is_some_and(Option::is_some)
                        || direct_lexical_loan_root(place).is_none()
                        || original_points[id.0].successors.as_slice().first()
                            .filter(|_| original_points[id.0].successors.len() == 1)
                            .is_some_and(|successor| {
                                let next = &original_points[successor.0];
                                let moved = match &next.kind {
                                    CfgPointKind::Move { place, ty: ScalarType::Struct(_) | ScalarType::Array { .. }
                                        | ScalarType::RuntimeArray { .. }, .. } => place,
                                    CfgPointKind::DeferredMove { place, .. }
                                        if deferred_policies.get(&successor.0) == Some(&CopyPolicy::Move) => place,
                                    _ => return false,
                                };
                                point.scope == next.scope && point.source_span == next.source_span
                                    && reference_places_same(moved, place)
                            })
                    {
                        continue;
                    }
                    let staged = if !state.projected.live_loans && place.projections.is_empty()
                        && matches!(&point.kind, CfgPointKind::Read { .. }) {
                        cfg.local_borrow.and_then(|sealed| {
                            if incoming_count == 1 {
                                sealed.conflicts(&place.binding, id, incoming_count, original_points, local_borrow::AccessMode::Shared)
                            } else {
                                local_access_conflicts.get_or_insert_with(||
                                    sealed.conflicts(&place.binding, id, incoming_count, original_points, local_borrow::AccessMode::Shared)).clone()
                            }
                        })
                    } else {
                        None
                    };
                    #[cfg(test)]
                    if incoming_count > 1 && staged.as_ref().is_some_and(|spans| !spans.is_empty()) {
                        local_borrow::tests::observe(4);
                    }
                    let origins = staged.unwrap_or_else(|| conflicting_live_loan(
                        &liveness, &cfg.facts, &state,
                        id, place, LoanAccess::DirectShared,
                    ));
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace { source_span: point.source_span.clone(), origins });
                    }
                }
            }
            if let CfgPointKind::Borrow { place, .. } = &point.kind {
                if let Some(bindings) = cfg.fallible_allocation_bindings {
                    if bindings.get(&place.binding).is_some_and(|site| site.has_indexed_extent()) {
                        require_available_fallible_read(bindings, &state, &place.binding, point, false, &mut errors);
                    }
                }
            }
            for place in read_places {
                if let Some(bindings) = cfg.fallible_allocation_bindings {
                    if matches!(&point.kind, CfgPointKind::ReadPlace { pointer_point: None, .. })
                        && (is_lexical_field_path(&place.projections)
                            || bindings.get(&place.binding).is_some_and(|owner| owner.has_indexed_extent()))
                    {
                        require_available_fallible_read(bindings, &state, &place.binding, point, false, &mut errors);
                    }
                }
                let normalized = available_place_for_check(place);
                if let Some(storage) = handle_storage {
                    let storage_place = if matches!(storage, CheckedHandleStorage::Pointee) {
                        normalized.as_ref()
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
                let origins = unavailable_origins_for_normalized_place(&state, &normalized);
                if !origins.is_empty()
                    || state.released.contains(&place.binding)
                    || state.invalidated.contains(&place.binding)
                {
                    errors.push(ReferenceAnalysisError::ConflictingPlace {
                        source_span: point.source_span.clone(),
                        origins,
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
                    let mut origins = unavailable_origins_for_normalized_place(&state, &place);
                    origins.extend(conflicting_live_loan(
                        &liveness, &cfg.facts,
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
                    if guarded_move_points.get(&id) == Some(&&place.binding) && place.projections.is_empty() {
                        state.guarded_whole_moves.entry(place.binding.clone()).or_insert_with(|| point.source_span.clone());
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
                        let mut origins = unavailable_origins_for_normalized_place(&state, &place);
                        origins.extend(conflicting_live_loan(
                            &liveness, &cfg.facts,
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
                    let widened_rebind = cfg.widened_rebind_points.is_some_and(|points| points.contains(&id));
                    if widened_rebind && (state.released.contains(&target.binding)
                        || state.invalidated.contains(&target.binding) || state.uninitialized.contains(&target.binding)
                        || state.moved.iter().any(|(place, _)| place.binding == target.binding))
                    {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: point.source_span.clone(),
                            reason: ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer,
                        });
                    }
                    if !widened_rebind && target.projections.is_empty()
                        && cfg.array_length_owners.is_some_and(|owners| owners.contains(&target.binding))
                    {
                        state.replaced_array_extents.insert(target.binding.clone());
                    }
                    if !target.projections.is_empty()
                        || (matches!(target.place, ScalarPlace::Name { .. }) && known_lexical_owners.as_ref()
                            .is_some_and(|owners| owners.contains_key(&target.binding)))
                        || (pending_allowed
                            && pending.is_some_and(|pending| deferred_local_owner_store(pending, point, original_points)))
                    {
                        let staged = cfg.local_borrow.filter(|_| !state.projected.live_loans).and_then(|sealed| {
                            if !target.projections.is_empty() || !matches!(&target.place, ScalarPlace::Name { .. }) {
                                return None;
                            }
                            let verdict = || sealed.conflicts(&target.binding, id, incoming_count, original_points, local_borrow::AccessMode::Exclusive);
                            if incoming_count == 1 { verdict() }
                            else { local_access_conflicts.get_or_insert_with(verdict).clone() }
                        });
                        let origins = staged.unwrap_or_else(|| conflicting_live_loan(
                            &liveness, &cfg.facts,
                            &state, id, &target, LoanAccess::Direct,
                        ));
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
                        if target.projections.is_empty()
                            && cfg.array_length_owners.is_some_and(|owners| owners.contains(&target.binding))
                        {
                            state.replaced_array_extents.insert(target.binding.clone());
                        }
                        let origins = conflicting_live_loan(
                            &liveness, &cfg.facts,
                            &state,
                            id,
                            &target,
                            LoanAccess::WriteThrough(candidate, projected_write),
                        );
                        if !origins.is_empty() {
                            errors.push(ReferenceAnalysisError::ConflictingPlace {
                                source_span: point.source_span.clone(),
                                origins,
                            });
                        }
                        if definite_assignment {
                            restore_definitely_written_place(&mut state, &target);
                        }
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
                        if !extend_projected_read_origins(&state, candidate, &mut read, indexed_holder(&candidate.binding)) {
                            errors.push(ReferenceAnalysisError::MissingOrigin {
                                source_span: point.source_span.clone(),
                                binding_id: candidate.binding.clone(),
                                reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            });
                        }
                    }
                    if read.is_empty() {
                        None
                    } else {
                        Some(read)
                    }
                }
                CfgPointKind::Read { binding } => {
                    if let Some(bindings) = cfg.fallible_allocation_bindings {
                        if let Some(owner) = cfg.fallible_aliases.and_then(|aliases| aliases.get(binding)) {
                            require_available_fallible_read(bindings, &state, owner, point, false, &mut errors);
                        } else {
                            let whole = !cfg.guarded_runtime_moves.and_then(|moves| moves.get(&id))
                                .is_some_and(|transfer| &transfer.source == binding);
                            require_available_fallible_read(bindings, &state, binding, point, whole, &mut errors);
                        }
                    }
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
                            Some(origins) => {
                                if let Some(expired) = origins.iter().find_map(|origin|
                                    expired_local_loan_at(&cfg.facts[origin.0], &point.source_span))
                                {
                                    errors.push(ReferenceAnalysisError::ConflictingPlace {
                                        source_span: point.source_span.clone(),
                                        origins: vec![expired],
                                    });
                                    None
                                } else {
                                    Some(origins.clone())
                                }
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
                    }
                }
                CfgPointKind::Bind { binding }
                    if cfg.fallible_allocation_bindings.is_some_and(|bindings| bindings.contains_key(binding)) =>
                {
                    state.uninitialized.insert(binding.clone());
                    None
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
                CfgPointKind::Assign { target, .. }
                    if cfg.fallible_aliases.is_some_and(|aliases| aliases.contains_key(&target.binding)) =>
                {
                    errors.push(ReferenceAnalysisError::MissingOrigin {
                        source_span: point.source_span.clone(),
                        binding_id: target.binding.clone(),
                        reason: ReferenceAnalysisErrorReason::UnsupportedFallibleAllocationAccess,
                    });
                    None
                }
                CfgPointKind::Assign { target, .. }
                    if let Some(site) = cfg.fallible_allocation_bindings.and_then(|bindings| bindings.get(&target.binding)) =>
                {
                    let unavailable = state.uninitialized.contains(&target.binding) || state.released.contains(&target.binding)
                        || state.invalidated.contains(&target.binding);
                    let checked_indexed_store = site.has_indexed_extent()
                        && cfg.guarded_indexed_stores.is_some_and(|stores| stores.contains(&id));
                    let error = if checked_indexed_store {
                        unavailable.then_some(ReferenceAnalysisErrorReason::UnavailableFallibleAllocation)
                    } else {
                        guarded_allocation_overwrite_error(
                            site.whole_copy,
                            target.projections.is_empty() && matches!(target.place, ScalarPlace::Name { .. }),
                            unavailable,
                        )
                    };
                    if let Some(reason) = error {
                        errors.push(ReferenceAnalysisError::MissingOrigin {
                            source_span: point.source_span.clone(),
                            binding_id: target.binding.clone(),
                            reason,
                        });
                    }
                    // An overwrite changes bytes, not the branch's proof of allocation
                    // presence or the original backing's cleanup responsibility.
                    None
                }
                CfgPointKind::Assign {
                    target,
                    previous_origins,
                    ..
                } => {
                    let binding = &target.binding;
                    let installs_reference = point.possible_origins.as_ref().is_none_or(|origins| !origins.is_empty());
                    let holder = indexed_holder(binding);
                    let holder_write = holder.filter(|holder|
                        installs_reference && (holder.selected(target) || holder.constant(target))
                            && assignment_rhs.as_ref().and_then(|value| value.as_ref()).is_some_and(|origins|
                                SelectedIndexedHolder::eligible_origins(origins, &cfg.facts))
                    );
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
                        retire_overwritten_binding_origins(&mut state, previous_origins);
                        if holder_write.is_none() {
                            state.projected.retain(|known| !reference_places_overlap(&known.place, target));
                        }
                    }
                    if installs_reference {
                        match assignment_rhs
                            .flatten()
                            .filter(|origins| !origins.is_empty())
                        {
                            Some(origins) => {
                                if target.projections.is_empty() {
                                    state.bindings.insert(binding.clone(), origins.clone());
                                } else if let Some(holder) = holder_write {
                                    holder.write(&mut state, target, &origins, &cfg.facts);
                                } else if !holder.is_some_and(|holder| holder.selected(target)) {
                                    let retains_loan = direct_lexical_loan_root(target).is_some()
                                        && exact_projected_prefix(target, target)
                                        && origins.iter().any(|origin| declared_local_loan(&cfg.facts[origin.0]).is_some());
                                    write_projected_reference(&mut state, target, &origins, retains_loan);
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
                    retire_overwritten_binding_origins(&mut state, previous_origins);
                    let rhs = assignment_rhs
                        .flatten()
                        .filter(|origins| !origins.is_empty());
                    let installs_reference = point.possible_origins.as_ref().is_none_or(|origins| !origins.is_empty());
                    let installs_local_loan = installs_reference
                        && rhs.as_ref().is_some_and(|origins| origins.iter().any(|origin|
                            declared_local_loan(&cfg.facts[origin.0]).is_some()));
                    update_projected_assignment(
                        &mut state,
                        candidates.iter().map(|candidate| (&candidate.place,
                            installs_local_loan
                                && candidate.loan.mode == ScalarReferenceMutability::Mutable
                                && direct_lexical_loan_root(&candidate.loan.origin_place).is_some())),
                        if installs_reference { rhs.as_ref() } else { None },
                        definite_assignment,
                        installs_local_loan,
                    );
                    if installs_reference && rhs.is_none() {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: point.source_span.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                        });
                    }
                    rhs
                }
                CfgPointKind::ReborrowThrough { mode, place, pointer_point } => 'reborrow: {
                    let admitted = joined_reborrow_parents.as_ref().and_then(|parents| {
                        let origins = state.point_values.get(*pointer_point)?;
                        origins.first().filter(|id| origins.len() == 1 && parents.contains(id)).copied()
                    });
                    let admitted = admitted.or_else(|| {
                        let family = joined_family.as_ref()?;
                        let (_, holder) = family.shape.descendants.get(&id)?;
                        family.live_authorized_origin(original_points, cfg.facts, &state, holder, *pointer_point)
                    });
                    let Some(parent_id) = admitted else {
                        let reason = ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin;
                        errors.push(match &original_points[pointer_point.0].kind {
                            CfgPointKind::Read { binding } => ReferenceAnalysisError::MissingOrigin {
                                source_span: point.source_span.clone(), binding_id: binding.clone(), reason,
                            },
                            _ => ReferenceAnalysisError::MissingContext { source_span: point.source_span.clone(), reason },
                        });
                        break 'reborrow None;
                    };
                    let parent_fact = &cfg.facts[parent_id.0];
                    let ReferenceOrigin::Fresh { loan: parent, .. } = parent_fact else { unreachable!() };
                    let mut origin_place = parent.origin_place.clone();
                    origin_place.place = place.clone();
                    origin_place.projections.clear();
                    collect_reference_projections(place, Some(parent), &mut origin_place.projections);
                    let loan = ReferenceLoanId {
                        mode: *mode,
                        origin_place,
                        creation_span: point.source_span.range,
                        parent: Some(Box::new(parent.clone())),
                    };
                    let origins = conflicting_live_loan(
                        &liveness, &cfg.facts,
                        &state, id, &loan.origin_place, LoanAccess::Borrow(&loan),
                    );
                    if !origins.is_empty() {
                        errors.push(ReferenceAnalysisError::ConflictingPlace {
                            source_span: point.source_span.clone(), origins,
                        });
                        break 'reborrow None;
                    }
                    let child = deferred_reborrow_child(parent_fact, loan);
                    let child = if let Some(index) = cfg.provisional_origin_index.as_deref_mut() {
                        index.intern(cfg.facts, child)
                    } else if let Some(index) = cfg.facts.iter().position(|fact| *fact == child) {
                        ReferenceOriginId(index)
                    } else {
                        errors.push(ReferenceAnalysisError::MissingContext {
                            source_span: point.source_span.clone(),
                            reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                        });
                        break 'reborrow None;
                    };
                    Some(BTreeSet::from([child]))
                }
                CfgPointKind::Call { .. } if joined_call.is_some() => None,
                CfgPointKind::Evaluate(_)
                | CfgPointKind::Borrow { .. }
                | CfgPointKind::Call { .. } => 'value: {
                    if let CfgPointKind::Borrow { place, loan } = &point.kind {
                        let check = if projected_loan.is_some() {
                            if projected_rejected {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                    source_span: point.source_span.clone(),
                                    reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                                });
                                point.possible_origins = None;
                                break 'value None;
                            }
                            true
                        } else {
                            if state.released.contains(&place.binding) {
                                errors.push(ReferenceAnalysisError::MissingContext {
                                    source_span: point.source_span.clone(),
                                    reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                                });
                            }
                            if loan.parent.is_none() {
                                direct_lexical_loan_root(place).is_some()
                            } else {
                                pending_here.is_some_and(|(_, site)| selected_parent.is_some_and(|(origin, _)|
                                    state.point_values.get(site.read).is_some_and(|origins| origins.contains(&origin))))
                                    || named_parent().is_some_and(|parent| state.point_values.get(parent.pointer)
                                        .is_some_and(|origins| origins.contains(&parent.origin)))
                            }
                        };
                        if check && (!state.bindings.is_empty() || state.projected.live_loans) {
                            let access = if loan.parent.is_none()
                                && matches!(reference_root(&place.place), ScalarPlace::Name { .. })
                            {
                                match loan.mode {
                                    ScalarReferenceMutability::Shared => LoanAccess::DirectShared,
                                    ScalarReferenceMutability::Mutable => LoanAccess::Direct,
                                }
                            } else {
                                LoanAccess::Borrow(loan)
                            };
                            let staged = cfg.local_borrow.filter(|_| !state.projected.live_loans).and_then(|sealed| {
                                let CfgPointKind::Borrow { place: original, loan: issuance } =
                                    &original_points[id.0].kind else { return None };
                                if original != place || issuance != loan || loan.origin_place != *place
                                    || loan.parent.is_some() || !place.projections.is_empty()
                                    || !matches!(&place.place, ScalarPlace::Name { .. })
                                {
                                    return None;
                                }
                                let mode = match loan.mode {
                                    ScalarReferenceMutability::Shared => local_borrow::AccessMode::Shared,
                                    ScalarReferenceMutability::Mutable => local_borrow::AccessMode::Exclusive,
                                };
                                let verdict = || sealed.conflicts(&place.binding, id, incoming_count, original_points, mode);
                                if incoming_count == 1 { verdict() }
                                else { local_access_conflicts.get_or_insert_with(verdict).clone() }
                            });
                            let origins = staged.unwrap_or_else(|| conflicting_live_loan(
                                &liveness, &cfg.facts,
                                &state, id, place, access,
                            ));
                            if !origins.is_empty() {
                                errors.push(ReferenceAnalysisError::ConflictingPlace {
                                    source_span: point.source_span.clone(),
                                    origins,
                                });
                            }
                        }
                    }
                    if let CfgPointKind::Call {
                        target: ResolvedCallTarget::Core(CoreOperationId::ArrayLen), argument_points, ..
                    } = &point.kind {
                        // The address is the already-authored shared Borrow, not a
                        // reconstructed name. Observing metadata requires the
                        // original whole backing to remain live on this edge.
                        let live = matches!(argument_points.as_slice(), [argument]
                            if matches!(&original_points[argument.0].kind, CfgPointKind::Borrow { place, .. }
                                if !state.released.contains(&place.binding)
                                    && !state.invalidated.contains(&place.binding)
                                    && !state.uninitialized.contains(&place.binding)
                                    && (!state.replaced_array_extents.contains(&place.binding)
                                        || cfg.pending_returned_extent == Some(id))
                                    && !state.moved.iter().any(|(moved, _)| moved.binding == place.binding
                                        && moved.projections.is_empty())));
                        if !live {
                            errors.push(ReferenceAnalysisError::MissingContext {
                                source_span: point.source_span.clone(),
                                reason: ReferenceAnalysisErrorReason::UnprovedArrayLength,
                            });
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
            let checked_value = match &point.kind {
                CfgPointKind::ReturnOutput { .. } | CfgPointKind::Return => {
                    point.possible_origins = checked_value;
                    None
                }
                kind => {
                    if matches!(
                        kind,
                        CfgPointKind::Read { .. }
                            | CfgPointKind::ReadPlace {
                                reads_origin: true,
                                ..
                            }
                            | CfgPointKind::Bind { .. }
                            | CfgPointKind::Assign { .. }
                            | CfgPointKind::AssignThrough { .. }
                            | CfgPointKind::ReborrowThrough { .. }
                    ) || checked_join_values.is_some() || checked_block_value.is_some() || point.possible_origins.is_none()
                    {
                        point.possible_origins = checked_value.clone();
                    }
                    checked_value
                }
            };
            if let Some(descriptor) = descriptor.as_deref_mut() {
                descriptor.observe(point, &state, &cfg.facts);
            }
            match &point.kind {
                CfgPointKind::ScopeExit => state.expire_scope(point, cfg.scope_exit_bindings, &cfg.facts),
                CfgPointKind::Read { .. }
                | CfgPointKind::ReadPlace { .. }
                | CfgPointKind::Join
                | CfgPointKind::Evaluate(_)
                | CfgPointKind::Call { .. }
                | CfgPointKind::Borrow { .. }
                | CfgPointKind::ReborrowThrough { .. }
                | CfgPointKind::Bind { .. }
                | CfgPointKind::Assign { .. } => state.value = checked_value,
                CfgPointKind::AssignThrough { .. } => state.value = checked_value,
                _ => {}
            }
            state
                .point_values
                .record(id, &point.possible_origins, &last_use);
            if let Some(previous) = snapshot.as_deref_mut().and_then(Option::as_mut) {
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
            } else if let Some(snapshot) = snapshot.as_deref_mut() {
                *snapshot = Some(point.clone());
            }
            if let (Some(cutoff), Some(last_use), Some(flow)) =
                (common_aggregate_cutoff, &aggregate_deadlines, &mut state.aggregate)
            {
                flow.expire_before(cutoff, last_use);
            }
            if let Some(edge) = guarded_allocation_edge.filter(|edge| state.released.contains(&edge.binding)) {
                errors.push(ReferenceAnalysisError::MissingOrigin { source_span: point.source_span.clone(),
                    binding_id: edge.binding.clone(), reason: ReferenceAnalysisErrorReason::UnavailableFallibleAllocation });
            }
            if enrolled_move_owner {
                let edge = guarded_allocation_edge.expect("enrolled guarded owner");
                if let Some(origin) = state.guarded_whole_moves.get(&edge.binding) {
                    errors.push(ReferenceAnalysisError::ConflictingPlace {
                        source_span: point.source_span.clone(), origins: vec![origin.clone()],
                    });
                }
            }
            let prune_for_successor = |next: &mut ReferenceEdgeState, successor| {
                if let Some(edge) = guarded_allocation_edge {
                    next.refine_allocation_edge(edge, successor, enrolled_move_owner);
                }
                if common_aggregate_cutoff.is_some_and(|cutoff| cutoff < successor) {
                    if let (Some(last_use), Some(flow)) = (&aggregate_deadlines, &mut next.aggregate) {
                        flow.expire_before(successor, last_use);
                    }
                }
            };
            let single_successor = point.successors.len() == 1;
            let mut state = Some(state);
            let mut successors = point.successors.iter().peekable();
            while let Some(successor) = successors.next() {
                let slot = incoming.slot(*successor);
                let final_successor = successors.peek().is_none();
                if move_alternatives.is_empty() {
                    let mut next = forwarded_edge_seed(&mut state, final_successor);
                    if successor.0 <= id.0 {
                        next.point_values.clear_from(*successor);
                    }
                    prune_for_successor(&mut next, *successor);
                    if fallback.as_ref().is_none_or(|(visited, _, _)| !visited[successor.0].contains(&next))
                        && !slot.contains(&next) {
                        slot.push(next);
                    }
                } else {
                    let mut seen_unchanged = false;
                    let mut alternatives = move_alternatives.iter().peekable();
                    while let Some((place, origin)) = alternatives.next() {
                        let effect = forwarded_move(
                            &state.as_ref().expect("seed for successor alternatives").moved,
                            place, &mut seen_unchanged, single_successor,
                        );
                        if effect == ForwardedMove::DuplicateUnchanged {
                            continue;
                        }
                        let final_edge = final_successor && alternatives.peek().is_none();
                        let mut next = forwarded_edge_seed(&mut state, final_edge);
                        if effect == ForwardedMove::Append {
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
    drop(incoming);
    for (point, observed) in cfg.points.iter_mut().zip(observed.into_iter().flatten()) {
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
    duplicate_sources: HashSet<&'a SourceIdentity>,
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
            match index.modules.entry(module.source) {
                std::collections::hash_map::Entry::Vacant(entry) => { entry.insert(headers); }
                std::collections::hash_map::Entry::Occupied(_) => { index.duplicate_sources.insert(module.source); }
            }
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
    let mut calls = resolve_module_call_sites(
        &program.source,
        &program.items,
        &[],
        &declarations,
        program.resolved_assignment_outputs.as_map(),
        select_executable_calls,
        occurrence_typing,
    )?;
    ground_shared_reader_forwarders(std::iter::once((&program.source, &mut calls)));
    Ok(calls)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SharedScalarReadEffect {
    parameter: usize,
    // Descriptor projections authorize only this exact direct receipt read.
    descriptor: bool,
    // Structured bodies and transitive wrappers authorize only exact joined observations.
    joined_only: bool,
    bool_prefix: bool,
    returns_formal: bool,
}

// A small, total structural pass for one typed conditional-reader body.
// Ordinary unary readers retain their existing, narrower legacy certificate.
fn exact_joined_conditional_reader(
    expression: &ScalarExpression,
    ordinal: OccurrenceOrdinal,
    structure: &OccurrenceStructure,
    parameter: &ParameterSlot,
    ordinary_formal: Option<&ParameterSlot>,
    inner: &ScalarType,
    formal_name: &impl Fn(OccurrenceOrdinal, ByteSpan) -> bool,
    mut remaining: usize,
) -> Option<()> {
    let mut pending = vec![(expression, ordinal)];
    while let Some((expression, ordinal)) = pending.pop() {
        remaining = remaining.checked_sub(1)?;
        let ty = structure.checked_types.expressions.get(&ordinal)?;
        if ty != inner && *ty != ScalarType::Bool { return None; }
        let parent = OccurrenceParent::Expression(ordinal);
        let child = |edge| match structure.edges.get(&(parent, edge))? {
            OccurrenceNode::Expression(child) => Some(*child),
            _ => None,
        };
        match expression {
            ScalarExpression::Integer { .. } if ty == inner => {},
            ScalarExpression::Boolean { .. } if *ty == ScalarType::Bool => {},
            ScalarExpression::Name { .. } if *ty == ScalarType::Bool
                && ordinary_formal.is_some_and(|formal| formal_name(ordinal, formal.span)) => {},
            ScalarExpression::Dereference { place: ScalarPlace::Dereference { pointer, .. }, .. }
                if ty == inner && matches!(pointer.as_ref(), ScalarExpression::Name { .. }) =>
            {
                let OccurrenceNode::Place(place) = *structure.edges.get(&(parent, OccurrenceEdge::ExpressionPlace))?
                    else { return None };
                let OccurrenceNode::Expression(pointer) = *structure.edges.get(&(
                    OccurrenceParent::Place(place), OccurrenceEdge::DereferencePointer,
                ))? else { return None };
                if !formal_name(pointer, parameter.span)
                    || structure.checked_types.expressions.get(&pointer) != Some(&parameter.ty)
                {
                    return None;
                }
            }
            ScalarExpression::Binary {
                operator: BinaryOperator::Add | BinaryOperator::Subtract | BinaryOperator::Multiply
                    | BinaryOperator::Equal | BinaryOperator::NotEqual | BinaryOperator::Less
                    | BinaryOperator::LessEqual | BinaryOperator::Greater | BinaryOperator::GreaterEqual,
                left, right, ..
            } => {
                pending.push((right.as_ref(), child(OccurrenceEdge::BinaryRight)?));
                pending.push((left.as_ref(), child(OccurrenceEdge::BinaryLeft)?));
            }
            ScalarExpression::If { condition, then_branch, else_branch, .. } => {
                let condition_node = child(OccurrenceEdge::Condition)?;
                if structure.checked_types.expressions.get(&condition_node) != Some(&ScalarType::Bool) {
                    return None;
                }
                for (branch, edge) in [(else_branch, OccurrenceEdge::ElseBranch), (then_branch, OccurrenceEdge::ThenBranch)] {
                    let [ScalarBlockItem::Expression(result)] = branch.items.as_slice() else { return None };
                    let OccurrenceNode::Block(block) = *structure.edges.get(&(parent, edge))? else { return None };
                    let OccurrenceNode::Expression(output) = *structure.edges.get(&(
                        OccurrenceParent::Block(block), OccurrenceEdge::FinalOutput { item: 0, output: 0 },
                    ))? else { return None };
                    if structure.checked_types.expressions.get(&output) != Some(ty) { return None; }
                    pending.push((result, output));
                }
                pending.push((condition.as_ref(), condition_node));
            }
            _ => return None,
        }
    }
    Some(())
}

impl SharedScalarReadEffect {
    fn prepare(
        source: &SourceIdentity, function: &ScalarFunction, calls: &FunctionCallSites,
    ) -> Option<Self> {
        let (scalar, parameter) = match calls.boundary.parameters.as_slice() {
            [parameter] => (None, parameter),
            [scalar, parameter] => (Some(scalar), parameter),
            _ => return None,
        };
        let ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Shared, inner,
        } = &parameter.ty else { return None };
        if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty()
        {
            return None;
        }
        let [result] = calls.boundary.results.as_slice() else { return None };
        let descriptor = matches!(inner.as_ref(), ScalarType::Struct(_));
        let [ScalarBlockItem::Expression(expression)] = function.body.items.as_slice() else {
            return None;
        };
        let structure = &calls.structure;
        let OccurrenceNode::Expression(output) = *structure.edges.get(&(
            OccurrenceParent::Block(structure.root),
            OccurrenceEdge::FinalOutput { item: 0, output: result.ordinal },
        ))? else { return None };
        let formal_name = |occurrence, span| {
            matches!(calls.prepared_plain_name(occurrence), Some(PlainNameIdentity::Lexical(binding))
                if binding.source == *source && binding.function_span == function.span
                    && binding.block_span == function.body.span && binding.declaration_span == span
                    && binding.kind == ReferenceBindingKind::Declared)
        };
        if scalar.is_none() && result.ty == parameter.ty && aggregate::ordinary_value(inner)
            && calls.call_occurrences.is_empty() && matches!(expression, ScalarExpression::Name { .. })
            && formal_name(output, parameter.span)
            && structure.checked_types.expressions.get(&output) == Some(&result.ty)
        {
            return Some(Self { parameter: parameter.ordinal, descriptor: false, joined_only: false, bool_prefix: false, returns_formal: true });
        }
        if if descriptor { !aggregate::ordinary_value(&result.ty) || !calls.call_occurrences.is_empty() }
            else { &result.ty != inner.as_ref() || !aggregate::ordinary_value(inner) } {
            return None;
        }
        if scalar.is_none_or(|formal| formal.ty == ScalarType::Bool)
            && !descriptor && matches!(expression, ScalarExpression::If { .. })
            && calls.call_occurrences.is_empty()
        {
            const MAX_CONDITIONAL_READER_NODES: usize = 32;
            exact_joined_conditional_reader(expression, output, structure, parameter, scalar,
                inner, &formal_name, MAX_CONDITIONAL_READER_NODES)?;
            return Some(Self { parameter: parameter.ordinal, descriptor: false,
                joined_only: true, bool_prefix: scalar.is_some(), returns_formal: false });
        }
        if scalar.is_some_and(|scalar| &scalar.ty != inner.as_ref() || !aggregate::ordinary_value(&scalar.ty)) {
            return None;
        }
        let (expression, read) = match (scalar, expression) {
            (Some(scalar), ScalarExpression::Binary { operator: BinaryOperator::Add, left, right, .. })
                if !descriptor && calls.call_occurrences.is_empty() =>
            {
                let (operand, dereference, ordinary) = match (left.as_ref(), right.as_ref()) {
                    (ScalarExpression::Name { .. }, operand @ ScalarExpression::Dereference { .. }) =>
                        (operand, OccurrenceEdge::BinaryRight, OccurrenceEdge::BinaryLeft),
                    (operand @ ScalarExpression::Dereference { .. }, ScalarExpression::Name { .. }) =>
                        (operand, OccurrenceEdge::BinaryLeft, OccurrenceEdge::BinaryRight),
                    _ => return None,
                };
                let child = |edge| match structure.edges.get(&(OccurrenceParent::Expression(output), edge))? {
                    OccurrenceNode::Expression(child) => Some(*child),
                    _ => None,
                };
                let ordinary = child(ordinary)?;
                if !formal_name(ordinary, scalar.span)
                    || structure.checked_types.expressions.get(&ordinary) != Some(&scalar.ty)
                    || structure.checked_types.expressions.get(&output) != Some(&result.ty)
                {
                    return None;
                }
                (operand, child(dereference)?)
            }
            (Some(_), _) => return None,
            (None, ScalarExpression::Binary {
                operator: BinaryOperator::Add | BinaryOperator::Subtract | BinaryOperator::Multiply,
                left, right, ..
            }) if !descriptor && calls.call_occurrences.is_empty()
                && structure.checked_types.expressions.get(&output) == Some(inner.as_ref()) =>
            {
                let (operand, edge) = match (left.as_ref(), right.as_ref()) {
                    (operand @ ScalarExpression::Dereference { .. }, ScalarExpression::Integer { .. }) =>
                        (operand, OccurrenceEdge::BinaryLeft),
                    (ScalarExpression::Integer { .. }, operand @ ScalarExpression::Dereference { .. }) =>
                        (operand, OccurrenceEdge::BinaryRight),
                    _ => return None,
                };
                let OccurrenceNode::Expression(read) = *structure.edges.get(&(
                    OccurrenceParent::Expression(output), edge,
                ))? else { return None };
                (operand, read)
            }
            (None, _) => (expression, output),
        };
        let ScalarExpression::Dereference { place: written, .. } = expression else { return None };
        let OccurrenceNode::Place(place) = *structure.edges.get(&(
            OccurrenceParent::Expression(read), OccurrenceEdge::ExpressionPlace,
        ))? else { return None };
        let (dereference, place) = if descriptor {
            let ScalarType::Struct(expected) = inner.as_ref() else { return None };
            let ScalarPlace::Field { base, field: ScalarFieldReference::Resolved(field), .. } = written else { return None };
            if &field.structure != expected { return None }
            let OccurrenceNode::Place(base_ordinal) = *structure.edges.get(&(
                OccurrenceParent::Place(place), OccurrenceEdge::PlaceBase,
            ))? else { return None };
            (base.as_ref(), base_ordinal)
        } else {
            (written, place)
        };
        let ScalarPlace::Dereference { pointer, .. } = dereference else { return None };
        if !matches!(pointer.as_ref(), ScalarExpression::Name { .. }) { return None }
        let OccurrenceNode::Expression(pointer) = *structure.edges.get(&(
            OccurrenceParent::Place(place), OccurrenceEdge::DereferencePointer,
        ))? else { return None };
        (formal_name(pointer, parameter.span)
            && structure.checked_types.expressions.get(&pointer) == Some(&parameter.ty)
            && structure.checked_types.expressions.get(&read) == Some(&result.ty))
            .then_some(Self { parameter: parameter.ordinal, descriptor, joined_only: false, bool_prefix: false, returns_formal: false })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ModuleCallSites {
    functions: BTreeMap<usize, FunctionCallSites>,
    shared_scalar_read_effects: HashMap<ByteSpan, SharedScalarReadEffect>,
    shared_reader_forwarders: Vec<(ByteSpan, SourceSpan, SharedForwardKind)>,
    exclusive_writer_declarations: HashSet<ByteSpan>,
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
    ValueFieldReceiver,
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
    proven_index_values: HashMap<OccurrenceOrdinal, u64>,
    pub(super) literal_fields: HashMap<OccurrenceOrdinal, ScalarStructFieldId>,
    pub(super) value_fields: HashMap<OccurrenceOrdinal, ScalarStructFieldId>,
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
            proven_index_values: HashMap::new(),
            literal_fields: HashMap::new(),
            value_fields: HashMap::new(),
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
    imported_shared_readers: HashMap<OccurrenceOrdinal, (SourceSpan, SharedScalarReadEffect)>,
    pub(super) local_shared_call_families: HashMap<OccurrenceOrdinal, bool>,
    exclusive_scalar_writers: HashSet<OccurrenceOrdinal>,
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
    fn attach_imported_reference_effects(
        &mut self, caller: &SourceIdentity,
        readers: &HashMap<&SourceIdentity, HashMap<ByteSpan, SharedScalarReadEffect>>,
        writers: &HashMap<&SourceIdentity, HashSet<ByteSpan>>,
    ) {
        for call in &self.call_occurrences {
            let Some(ResolvedCallTarget::ModuleCallable {
                source, declaration_span, concrete: ConcreteCallSelection::Declaration,
            }) = self.sites.get(call.site).map(|site| &site.target) else { continue };
            if source == caller || self.declared_unsafe_calls.contains(&call.ordinal.0) {
                continue;
            }
            if let Some(&effect) = readers.get(source).and_then(|readers| readers.get(declaration_span)) {
                self.imported_shared_readers.insert(
                    call.ordinal, (SourceSpan::new(source.clone(), *declaration_span), effect),
                );
            }
            if writers.get(source).is_some_and(|writers| writers.contains(declaration_span)) {
                self.exclusive_scalar_writers.insert(call.ordinal);
            }
        }
        for arm in &mut self.arms {
            arm.attach_imported_reference_effects(caller, readers, writers);
        }
    }

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
    duplicate_sources: HashSet<SourceIdentity>,
    modules: Vec<Result<ModuleCallSites, ReferenceAnalysisError>>,
}

#[cfg(test)]
impl ProjectCallSiteResults {
    pub(super) fn module(&self, ordinal: usize) -> &ModuleCallSites {
        self.modules[ordinal].as_ref().unwrap()
    }
}

impl ProjectCallSiteResults {
    pub(super) fn has_closed_shared_scalar_reader(&self, module: usize, declaration: ByteSpan) -> bool {
        self.modules.get(module).and_then(|row| row.as_ref().ok())
            .is_some_and(|calls| calls.shared_scalar_read_effects.get(&declaration)
                .is_some_and(|effect| effect.parameter == 0 && !effect.descriptor && !effect.joined_only && !effect.returns_formal))
    }

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
    let mut result = ProjectCallSiteResults {
        duplicate_sources: declarations.duplicate_sources.iter().map(|source| (*source).clone()).collect(),
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
    };
    ground_shared_reader_forwarders(modules.iter().zip(&mut result.modules).filter_map(|(module, calls)| {
        (!result.duplicate_sources.contains(&module.source)).then_some(())?;
        Some((&module.source, calls.as_mut().ok()?))
    }));
    // Only successfully prepared module rows may grant an imported exact
    // effect certificate. Duplicate full identities are never authority.
    let mut readers = HashMap::new();
    let mut writers = HashMap::new();
    for (module, calls) in modules.iter().zip(&mut result.modules) {
        if result.duplicate_sources.contains(&module.source) {
            continue;
        }
        let Ok(calls) = calls else { continue };
        readers.insert(&module.source, calls.shared_scalar_read_effects.clone());
        if !calls.exclusive_writer_declarations.is_empty() {
            writers.insert(&module.source, std::mem::take(&mut calls.exclusive_writer_declarations));
        }
    }
    for (module, calls) in modules.iter().zip(&mut result.modules) {
        if let Ok(calls) = calls {
            for function in calls.functions.values_mut() {
                function.attach_imported_reference_effects(&module.source, &readers, &writers);
            }
        }
    }
    result
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

// Deliberately tiny and total: no host overflow, no runtime effect or
// width conversion, and no cost proportional to large arithmetic trees.
fn closed_u64_constant(
    structure: &OccurrenceStructure,
    expression: &ScalarExpression,
    occurrence: OccurrenceOrdinal,
) -> Option<u64> {
    fn evaluate(
        structure: &OccurrenceStructure,
        expression: &ScalarExpression,
        occurrence: OccurrenceOrdinal,
        remaining: &mut u8,
    ) -> Option<u64> {
        *remaining = remaining.checked_sub(1)?;
        if structure.checked_types.expressions.get(&occurrence) != Some(&ScalarType::U64) {
            return None;
        }
        match expression {
            ScalarExpression::Integer { value, .. } => value.try_into().ok(),
            ScalarExpression::Binary { operator, left, right, .. } => {
                let child = |edge| match structure.edges.get(&(OccurrenceParent::Expression(occurrence), edge)) {
                    Some(OccurrenceNode::Expression(child)) => Some(*child),
                    _ => None,
                };
                let left = evaluate(structure, left, child(OccurrenceEdge::BinaryLeft)?, remaining)?;
                let right = evaluate(structure, right, child(OccurrenceEdge::BinaryRight)?, remaining)?;
                match operator {
                    BinaryOperator::Add => left.checked_add(right),
                    BinaryOperator::Subtract => left.checked_sub(right),
                    BinaryOperator::Multiply => left.checked_mul(right),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    evaluate(structure, expression, occurrence, &mut 15)
}

struct StructuralWalker<'a, 'typing> {
    mode: WalkMode<'a>,
    occurrence_typing: &'typing mut PreparationOccurrenceTyping,
    checked_call_signatures: HashMap<OccurrenceOrdinal, CheckedCallSignature>,
    plain_names: PlainNameFacts,
    named_place_roots: NamedPlaceRootFacts,
    direct_destinations: DirectDestinationFacts,
    members: MemberFacts,
    allocation_length_unsafe_calls: BTreeSet<usize>,
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
    known_u64_locals: HashMap<(u32, u32), u64>,
    index_uses: Vec<(OccurrenceOrdinal, (u32, u32), u64)>,

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
            allocation_length_unsafe_calls: BTreeSet::new(),
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
            known_u64_locals: HashMap::new(),
            index_uses: Vec::new(),

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
                        let (target, declared_unsafe) = resolver.resolve(
                            expression,
                            names,
                            types,
                            self.checked_call_signatures.get(&ordinal),
                        )?;
                        if declared_unsafe {
                            self.allocation_length_unsafe_calls.insert(ordinal.0);
                        }
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
            ScalarExpression::ValueField { receiver, .. } => {
                self.expression(receiver, names, types, parent, OccurrenceEdge::ValueFieldReceiver)?;
                if let Some(id) = self.occurrence_typing.take_value_field(expression) {
                    self.structure.value_fields.insert(ordinal, id);
                }
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
                if matches!(storage_role, ModuleStorageRole::Address | ModuleStorageRole::Replace) {
                    if let Some(local) = names.get(name) {
                        let span = local.declaration_span;
                        self.known_u64_locals.remove(&(span.start, span.end));
                    }
                }

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
                let index_ordinal =
                    self.expression_at(index, names, types, parent, OccurrenceEdge::PlaceIndex)?;
                if matches!(self.mode, WalkMode::Callable(ref callable) if callable.prepared)
                    && self.structure.checked_types.expressions.get(&index_ordinal) == Some(&ScalarType::U64)
                {
                    if let ScalarExpression::Name { name, .. } = index.as_ref() {
                        if let Some(local) = names.get(name) {
                            let span = local.declaration_span;
                            let key = (span.start, span.end);
                            if let Some(&value) = self.known_u64_locals.get(&key) {
                                self.index_uses.push((index_ordinal, key, value));
                            }
                        }
                    } else if !matches!(index.as_ref(), ScalarExpression::Integer { .. }) {
                        if let Some(value) = closed_u64_constant(&self.structure, index, index_ordinal) {
                            self.structure.proven_index_values.insert(index_ordinal, value);
                        }
                    }
                }
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
        if !binding.is_allocation && binding.fallible_assignment.is_none()
            && matches!(self.mode, WalkMode::Callable(ref callable) if callable.prepared)
        {
            if let [receiver] = binding.receivers.as_slice() {
                if receiver.ty == ScalarType::U64 {
                    if let BindingInitialization::Outputs(outputs) = &initialization {
                        if self.structure.checked_types.expressions.get(&outputs.primary) == Some(&ScalarType::U64) {
                            if let Some(value) = closed_u64_constant(&self.structure, &binding.value, outputs.primary) {
                                let span = receiver.name_span;
                                self.known_u64_locals.insert((span.start, span.end), value);
                            }
                        }
                    }
                }
            }
        }
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

fn close_local_shared_call_families(
    source: &SourceIdentity, calls: &mut FunctionCallSites,
    readers: &HashMap<ByteSpan, SharedScalarReadEffect>,
) {
    let mut slots = HashMap::<&ReferenceBindingId, (bool, bool)>::new();
    let checked_calls: Vec<_> = calls.call_occurrences.iter().filter_map(|call| {
        let ResolvedCallTarget::LocalCallable(binding) = &calls.sites[call.site].target else { return None };
        let signature = calls.checked_call_signatures.get(&call.ordinal)?;
        let [ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner }] =
            signature.parameters.as_slice() else { return None };
        let [output] = signature.outputs.outputs.as_slice() else { return None };
        (output.ty == **inner && aggregate::ordinary_value(inner)
            && !calls.declared_unsafe_calls.contains(&call.ordinal.0))
            .then_some((call.ordinal, binding))
    }).collect();
    if checked_calls.is_empty() { return; }
    for &(_, binding) in &checked_calls { slots.entry(binding).or_default(); }
    let by_declaration: HashMap<_, _> = slots.keys().map(|&binding| ((binding.block_span, binding.declaration_span), binding)).collect();
    let is_reader = |expression| {
        matches!(calls.prepared_plain_name(expression), Some(PlainNameIdentity::ModuleValue(declaration))
            if declaration.source == *source && readers.get(&declaration.range)
                .is_some_and(|effect| effect.parameter == 0 && !effect.descriptor && !effect.joined_only && !effect.returns_formal))
    };
    for block in &calls.structure.blocks {
        for item in &block.items {
            let BlockItemKind::Binding(binding) = &item.kind else { continue; };
            for receiver in binding.receivers() {
                let Some(&identity) = by_declaration.get(&(block.span, receiver.span)) else { continue; };
                let state = slots.get_mut(identity).expect("indexed local shared slot");
                if state.0 || binding.receivers().len() != 1
                    || !matches!(&binding.initialization, BindingInitialization::Outputs(outputs)
                        if outputs.secondary.is_empty() && is_reader(outputs.primary))
                {
                    state.1 = true;
                }
                state.0 = true;
            }
        }
    }
    // Every possible writer is closed, so at any reachable call the selected
    // definition is a member of this proved family. We do not infer a target
    // from the condition, nor model general callable values or aliases.
    for group in calls.assignments.values() {
        let mut destinations = group.destinations();
        let single = destinations.len() == 1 && group.projected_outputs().len() == 0;
        let mut rhs = group.rhs_evaluations();
        let accepted = single && rhs.len() == 1 && rhs.next().is_some_and(|value| is_reader(value.expression));
        for slot in &mut destinations {
            let Some(DirectDestinationIdentity::Existing(binding)) = calls.prepared_direct_destination(slot.destination)
                else { continue; };
            if let Some(state) = slots.get_mut(binding) { state.1 |= !accepted; }
        }
    }
    if let PlainNameFacts::Prepared(names) = &calls.plain_names {
        for identity in names.values() {
            if let PlainNameIdentity::Lexical(binding) = identity {
                if let Some(state) = slots.get_mut(binding) { state.1 = true; }
            }
        }
    }
    if let NamedPlaceRootFacts::Prepared(roots) = &calls.named_place_roots {
        for (&occurrence, identity) in roots {
            if let NamedPlaceRootIdentity::Lexical(binding) = identity {
                if let Some(state) = slots.get_mut(binding) {
                    state.1 |= !matches!(calls.prepared_direct_destination(occurrence),
                        Some(DirectDestinationIdentity::Existing(destination)) if destination == binding);
                }
            }
        }
    }
    calls.local_shared_call_families.extend(checked_calls.into_iter().filter_map(|(ordinal, binding)| {
        let &(initialized, rejected) = slots.get(binding)?;
        initialized.then_some((ordinal, !rejected))
    }));
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SharedForwardKind {
    ScalarRead,
    ReturnedFormal,
}

fn ground_shared_reader_forwarders<'a>(
    rows: impl Iterator<Item = (&'a SourceIdentity, &'a mut ModuleCallSites)>,
) {
    let mut rows: Vec<_> = rows.collect();
    let source_ids: HashMap<_, _> = rows.iter().enumerate().map(|(index, (source, _))| (*source, index)).collect();
    let mut reverse = HashMap::<(usize, ByteSpan, SharedForwardKind), Vec<(usize, ByteSpan)>>::new();
    let mut grounded = Vec::new();
    for (index, (_, calls)) in rows.iter_mut().enumerate() {
        for (declaration, target, kind) in std::mem::take(&mut calls.shared_reader_forwarders) {
            if let Some(&target_module) = source_ids.get(&target.source) {
                reverse.entry((target_module, target.range, kind)).or_default().push((index, declaration));
            }
        }
        grounded.extend(calls.shared_scalar_read_effects.iter().filter_map(|(&declaration, effect)| {
            (effect.parameter == 0 && !effect.descriptor).then_some((index, declaration,
                if effect.returns_formal { SharedForwardKind::ReturnedFormal } else { SharedForwardKind::ScalarRead }))
        }));
    }
    // Seed only exact unary leaves so cycles, failed rows and same-offset
    // declarations from another source cannot grant forwarding authority.
    while let Some(target) = grounded.pop() {
        if let Some(callers) = reverse.remove(&target) {
            for (index, declaration) in callers {
                rows[index].1.shared_scalar_read_effects.insert(declaration, SharedScalarReadEffect {
                    parameter: 0, descriptor: false, joined_only: true, bool_prefix: false,
                    returns_formal: target.2 == SharedForwardKind::ReturnedFormal,
                });
                grounded.push((index, declaration, target.2));
            }
        }
    }
}

fn shared_scalar_reader_forward_target(
    source: &SourceIdentity, function: &ScalarFunction, calls: &FunctionCallSites,
) -> Option<(SourceSpan, SharedForwardKind)> {
    let [parameter] = calls.boundary.parameters.as_slice() else { return None };
    let ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner } =
        &parameter.ty else { return None };
    let [result] = calls.boundary.results.as_slice() else { return None };
    if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty()
        || !aggregate::ordinary_value(inner)
        || !matches!(function.body.items.as_slice(), [ScalarBlockItem::Expression(ScalarExpression::Call { .. })])
    {
        return None;
    }
    let kind = if result.ty == **inner {
        SharedForwardKind::ScalarRead
    } else if result.ty == parameter.ty {
        SharedForwardKind::ReturnedFormal
    } else {
        return None;
    };
    let [call] = calls.call_occurrences.as_slice() else { return None };
    let structure = &calls.structure;
    let OccurrenceNode::Expression(output) = *structure.edges.get(&(
        OccurrenceParent::Block(structure.root),
        OccurrenceEdge::FinalOutput { item: 0, output: result.ordinal },
    ))? else { return None };
    if output != call.ordinal || calls.declared_unsafe_calls.contains(&output.0) {
        return None;
    }
    let ResolvedCallTarget::ModuleCallable { source: target, declaration_span,
        concrete: ConcreteCallSelection::Declaration } = &calls.sites.get(call.site)?.target else { return None };
    let signature = calls.checked_call_signatures.get(&output)?;
    let [result_output] = signature.outputs.outputs.as_slice() else { return None };
    if signature.parameters.as_slice() != std::slice::from_ref(&parameter.ty) || result_output.ty != result.ty
        || structure.checked_types.expressions.get(&output) != Some(&result.ty)
    {
        return None;
    }
    let OccurrenceNode::Expression(argument) = *structure.edges.get(&(
        OccurrenceParent::Expression(output), OccurrenceEdge::CallArgument(0),
    ))? else { return None };
    let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(argument)? else { return None };
    (binding.source == *source && binding.function_span == function.span
        && binding.block_span == function.body.span && binding.declaration_span == parameter.span
        && binding.kind == ReferenceBindingKind::Declared
        && structure.checked_types.expressions.get(&argument) == Some(&parameter.ty))
        .then(|| (SourceSpan::new(target.clone(), *declaration_span), kind))
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
    let mut shared_scalar_read_effects = HashMap::new();
    let mut exclusive_scalar_writers = HashSet::new();
    let mut shared_reader_forwarders = Vec::new();
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
                if exact_exclusive_scalar_writer(function, &calls) {
                    exclusive_scalar_writers.insert(function.name_span);
                }
                if let Some(effect) = SharedScalarReadEffect::prepare(source, function, &calls) {
                    shared_scalar_read_effects.insert(function.name_span, effect);
                } else if let Some((target, kind)) = shared_scalar_reader_forward_target(source, function, &calls) {
                    shared_reader_forwarders.push((function.name_span, target, kind));
                }
            }
            functions.insert(index, calls);
        }
    }
    if select_executable_calls {
        for calls in functions.values_mut() {
            close_local_shared_call_families(source, calls, &shared_scalar_read_effects);
            attach_local_exclusive_writers(calls, source, &exclusive_scalar_writers);
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
        shared_scalar_read_effects,
        shared_reader_forwarders,
        exclusive_writer_declarations: exclusive_scalar_writers,
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
    for (occurrence, binding, value) in resolver.index_uses {
        if resolver.known_u64_locals.get(&binding) == Some(&value) {
            resolver.structure.proven_index_values.insert(occurrence, value);
        }
    }
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
        imported_shared_readers: HashMap::new(),
        exclusive_scalar_writers: HashSet::new(),
        local_shared_call_families: HashMap::new(),
        declared_unsafe_calls: {
            let mut unsafe_calls = callable.declared_unsafe_calls;
            unsafe_calls.extend(resolver.allocation_length_unsafe_calls);
            unsafe_calls
        },
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

type MixedReceiptTargets = HashMap<SourceIdentity, HashSet<ByteSpan>>;
type MixedReceiptKey<'a> = (&'a SourceIdentity, ByteSpan);
type MixedReceiptEdges<'a> = HashMap<MixedReceiptKey<'a>, Vec<MixedReceiptKey<'a>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MixedReceiptProof {
    Local(aggregate::SealedMixedOwner),
    Forwarded,
    Multiple,
    MultipleForwarded,
}

type MixedReceiptRoots = HashMap<SourceIdentity, HashMap<ByteSpan, MixedReceiptProof>>;

#[derive(Default)]
struct MixedReceiptModuleProofs {
    local: HashMap<ByteSpan, MixedReceiptProof>,
    forwarded: HashSet<ByteSpan>,
}

// This is the only index caller replay accepts. Ordinary qualified edges
// cannot be used as receipt authority until resolve proves reachability.
struct MixedReceiptAuthority(HashMap<SourceIdentity, MixedReceiptModuleProofs>);

impl MixedReceiptAuthority {
    fn proof(&self, source: &SourceIdentity, span: &ByteSpan) -> Option<MixedReceiptProof> {
        let module = self.0.get(source)?;
        module.local.get(span).copied()
            .or_else(|| module.forwarded.contains(span).then_some(MixedReceiptProof::Forwarded))
    }

    fn contains(&self, source: &SourceIdentity, span: &ByteSpan) -> bool {
        matches!(self.proof(source, span), Some(MixedReceiptProof::Local(_) | MixedReceiptProof::Forwarded))
    }

    fn is_multi(&self, source: &SourceIdentity, span: &ByteSpan) -> bool {
        matches!(self.proof(source, span), Some(MixedReceiptProof::Multiple | MixedReceiptProof::MultipleForwarded))
    }

    fn is_multi_forwarder(&self, source: &SourceIdentity, span: &ByteSpan) -> bool {
        self.proof(source, span) == Some(MixedReceiptProof::MultipleForwarded)
    }

    fn is_forwarder(&self, source: &SourceIdentity, span: &ByteSpan) -> bool {
        self.proof(source, span) == Some(MixedReceiptProof::Forwarded)
    }
}

struct MixedReceiptPlan<'a> {
    // Nominate only already sealed body-local allocation or branch proofs.
    // Dependency edges themselves do not carry authority.
    roots: MixedReceiptRoots,
    users: MixedReceiptEdges<'a>,
}

impl MixedReceiptPlan<'_> {
    fn resolve(self) -> MixedReceiptAuthority {
        let Self { roots, users } = self;
        // Keep qualified edges borrowed while walking; only the final
        // per-module forwarding index needs owned source identities.
        fn rooted_forwarders<'a>(roots: &'a MixedReceiptRoots, mut users: MixedReceiptEdges<'a>) -> MixedReceiptTargets {
            if users.is_empty() { return HashMap::new(); }
            let mut pending: Vec<_> = roots.iter()
                .flat_map(|(source, proofs)| proofs.iter().filter_map(move |(span, proof)|
                    matches!(proof, MixedReceiptProof::Local(_)).then_some((source, *span)))).collect();
            let mut reached = HashSet::new();
            while let Some(callee) = pending.pop() {
                let Some(callers) = users.remove(&callee) else { continue };
                for caller in callers {
                    if !roots.get(caller.0).is_some_and(|proofs| proofs.contains_key(&caller.1))
                        && reached.insert(caller)
                    {
                        pending.push(caller);
                    }
                }
            }
            let mut forwarders = MixedReceiptTargets::new();
            for (source, span) in reached {
                if let Some(spans) = forwarders.get_mut(source) {
                    spans.insert(span);
                } else {
                    forwarders.insert(source.clone(), HashSet::from([span]));
                }
            }
            forwarders
        }

        // Only a relay directly grounded in a sealed local allocation result.
        let mut multiple = Vec::new();
        for (source, proofs) in &roots {
            for (declaration, proof) in proofs {
                if *proof == MixedReceiptProof::Multiple {
                    if let Some(callers) = users.get(&(source, *declaration)) {
                        multiple.extend(callers.iter().copied());
                    }
                }
            }
        }
        let mut forwarders = rooted_forwarders(&roots, users);
        let mut modules: HashMap<_, _> = roots.into_iter().map(|(source, local)| {
            let forwarded = forwarders.remove(&source).unwrap_or_default();
            (source, MixedReceiptModuleProofs { local, forwarded })
        }).collect();
        modules.extend(forwarders.into_iter().map(|(source, forwarded)| (source,
            MixedReceiptModuleProofs { forwarded, ..Default::default() })));
        for (source, declaration) in multiple {
            modules.entry(source.clone()).or_default().local.entry(declaration)
                .or_insert(MixedReceiptProof::MultipleForwarded);
        }
        MixedReceiptAuthority(modules)
    }
}

struct MixedReceiptCohort {
    module_names: HashSet<String>,
    callers: Vec<usize>,
    producers: Vec<(usize, ByteSpan, aggregate::MixedOwnerResultLanes)>,
    multi_producers: Vec<(usize, ByteSpan)>,
}

impl MixedReceiptCohort {
    fn grounds(
        &self, source: &SourceIdentity, items: &[ScalarItem], sites: &ModuleCallSites,
        updates: &mut [(usize, FunctionReferenceResult)], targets: &MixedReceiptTargets,
    ) -> HashMap<ByteSpan, MixedReceiptProof> {
        let Some(targets) = targets.get(source) else { return HashMap::new() };
        if self.producers.iter().any(|&(position, declaration, _)| targets.contains(&declaration)
            && updates[position].1.aggregate.can_seal_selected_mixed())
        {
            let make_body = |position: usize| {
                let (index, result) = updates.get(position)?;
                let ScalarItem::Function(function) = items.get(*index)? else { return None };
                Some(aggregate::Body {
                    facts: Some(&result.aggregate), cfg: &result.cfg, sites: sites.functions.get(index)?, source,
                    is_monomorphic: function.generic_parameters.is_empty() && function.overload_arms.is_empty(),
                    supports_input_result: false,
                })
            };
            let positions = updates.iter().enumerate().filter_map(|(position, (index, _))| {
                let ScalarItem::Function(function) = items.get(*index)? else { return None };
                Some((function.name_span, position))
            }).collect::<HashMap<_, _>>();
            let mut relay_slots = HashMap::new();
            let mut factories = HashMap::new();
            let lookup = |span: &ByteSpan| positions.get(span).and_then(|&position| make_body(position));
            let mut choices = Vec::new();
            for &(position, declaration, _) in &self.producers {
                if !targets.contains(&declaration) { continue; }
                let Some(body) = make_body(position) else { continue };
                if let Some(choice) = aggregate::checked_mixed_local_choice(body, |target| {
                    *relay_slots.entry(*target).or_insert_with(||
                        lookup(target).and_then(|body| body.exact_identity_relay_slot()))
                }, |target| {
                    *factories.entry(*target).or_insert_with(||
                        lookup(target).is_some_and(|body| body.is_exact_local_owner_factory()))
                }) {
                    choices.push((position, choice));
                }
            }
            for (position, choice) in choices {
                updates[position].1.aggregate.install_selected_mixed(choice);
            }
        }
        self.producers.iter().filter_map(|&(position, declaration, lanes)| {
            if !targets.contains(&declaration) { return None; }
            let result = &updates[position].1;
            let proof = result.aggregate.sealed_mixed_owner(&result.cfg, lanes)?;
            Some((declaration, MixedReceiptProof::Local(proof)))
        }).chain(self.multi_producers.iter().filter_map(|&(position, declaration)| {
            if !targets.contains(&declaration) { return None; }
            updates[position].1.aggregate.sealed_multi_result.as_ref()?;
            Some((declaration, MixedReceiptProof::Multiple))
        })).collect()
    }

    fn index_forwarders<'a>(
        &self, source: &'a SourceIdentity, items: &[ScalarItem], sites: &ModuleCallSites,
        updates: &'a [(usize, FunctionReferenceResult)], edges: &mut MixedReceiptEdges<'a>,
    ) {
        if self.callers.is_empty() {
            return;
        }
        for &(position, declaration, _) in &self.producers {
            let (index, result) = &updates[position];
            let ScalarItem::Function(function) = &items[*index] else { unreachable!("function result ordinal") };
            if let Some(callee) = result.mixed_forwarded_callee(function, &sites.functions[index]) {
                edges.entry(callee).or_default().push((source, declaration));
            }
        }
        for &(position, declaration) in &self.multi_producers {
            let (index, result) = &updates[position];
            let ScalarItem::Function(function) = &items[*index] else { unreachable!("function result ordinal") };
            if let Some(callee) = result.multi_forwarded_callee(function, &sites.functions[index]) {
                edges.entry(callee).or_default().push((source, declaration));
            }
        }
    }

    fn select(
        self, source: &SourceIdentity, items: &[ScalarItem], call_sites: &ModuleCallSites,
        updates: &mut [(usize, FunctionReferenceResult)], owners: &MixedReceiptAuthority,
        structs: &[ScalarStruct], enums: &[ScalarEnum], capture_call_points: bool,
        errors: &mut Vec<ReferenceAnalysisError>,
    ) {
        for position in self.callers {
            let (index, result) = &mut updates[position];
            let ScalarItem::Function(function) = &items[*index] else { unreachable!("function result ordinal") };
            result.observe_selected_mixed_receipts(function, &call_sites.functions[index], &self.module_names,
                structs, enums, source, &call_sites.shared_scalar_read_effects, owners, owners.is_forwarder(source, &function.name_span),
                capture_call_points, errors);
        }
    }
}

fn derive_module_reference_origins(
    source: &SourceIdentity, items: &[ScalarItem], call_sites: &ModuleCallSites,
    structs: &[ScalarStruct], enums: &[ScalarEnum], capture_call_points: bool,
) -> (Vec<(usize, FunctionReferenceResult)>, Vec<ReferenceAnalysisError>) {
    let mut targets = HashMap::new();
    let (mut updates, mut errors, cohort) =
        derive_module_reference_cohort(source, items, call_sites, structs, enums, capture_call_points, &mut targets);
    if errors.is_empty() {
        if let Some(cohort) = cohort {
            let owners = cohort.grounds(source, items, call_sites, &mut updates, &targets);
            if !owners.is_empty() {
                let grounds = HashMap::from([(source.clone(), owners)]);
                let mut edges = HashMap::new();
                cohort.index_forwarders(source, items, call_sites, &updates, &mut edges);
                let authority = MixedReceiptPlan { roots: grounds, users: edges }.resolve();
                cohort.select(source, items, call_sites, &mut updates, &authority, structs, enums, capture_call_points, &mut errors);
            }
        }
    }
    (updates, errors)
}

fn derive_module_reference_cohort(
    source: &SourceIdentity, items: &[ScalarItem], call_sites: &ModuleCallSites,
    structs: &[ScalarStruct], enums: &[ScalarEnum], capture_call_points: bool,
    mixed_targets: &mut MixedReceiptTargets,
) -> (Vec<(usize, FunctionReferenceResult)>, Vec<ReferenceAnalysisError>, Option<MixedReceiptCohort>) {
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
    let mut mixed_callers = Vec::new();
    let mut mixed_producers = Vec::new();
    let mut multi_producers = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if let ScalarItem::Function(function) = item {
            let result = derive_function_reference_result(
                source, function, &module_names, &call_sites.functions[&index],
                structs, enums, Some(&call_sites.shared_scalar_read_effects), capture_call_points, &mut errors,
            );
            let position = updates.len();
            if result.collect_mixed_targets(mixed_targets) {
                mixed_callers.push(position);
            }
            if function.generic_parameters.is_empty() && function.overload_arms.is_empty() {
                if let ScalarType::Callable { outputs, .. } = &function.signature {
                    if let Some(lanes) = aggregate::mixed_owned_struct_result(outputs) {
                        mixed_producers.push((position, function.name_span, lanes));
                    } else if outputs.outputs.len() > 1 && outputs.outputs.iter().all(|output| matches!(output.ty, ScalarType::Struct(_))) {
                        multi_producers.push((position, function.name_span));
                    }
                }
            }
            updates.push((index, result));
        }
    }
    let cohort = (!mixed_callers.is_empty() || !mixed_producers.is_empty() || !multi_producers.is_empty()).then_some(MixedReceiptCohort {
        module_names, callers: mixed_callers, producers: mixed_producers, multi_producers,
    });
    (updates, errors, cohort)
}

#[cfg(test)]
#[test]
fn mixed_wrapper_reachability_requires_an_allocation_grounded_predecessor() {
    let span = |index| ByteSpan::new(index, index + 1);
    let source = |path: &str, revision: &str| SourceIdentity::new(
        "project".into(), "payload".into(), path.into(), revision.into(),
    );
    let origin = source("owner.w", "1");
    let first = source("first.w", "1");
    let second = source("second.w", "1");
    let decoy = source("owner.w", "2");
    let lanes = aggregate::MixedOwnerResultLanes { owner: 1, scalar: 0 };
    let local = aggregate::SealedMixedOwner::Allocation { lanes, block: 2, item: 3 };
    let roots = HashMap::from([(origin.clone(), HashMap::from([(span(0), MixedReceiptProof::Local(local))]))]);
    let users = HashMap::from([
        ((&origin, span(0)), vec![(&first, span(0))]),
        ((&first, span(0)), vec![(&second, span(0)), (&second, span(1))]),
        ((&decoy, span(0)), vec![(&first, span(1))]),
        ((&first, span(4)), vec![(&second, span(4))]),
        ((&second, span(4)), vec![(&first, span(4))]),
        ((&second, span(5)), vec![(&second, span(5))]),
    ]);
    let actual = MixedReceiptPlan { roots, users }.resolve();
    assert_eq!(actual.0.values().map(|module| module.local.len() + module.forwarded.len()).sum::<usize>(), 4);
    assert_eq!(actual.proof(&origin, &span(0)), Some(MixedReceiptProof::Local(local)));
    assert!(!actual.is_forwarder(&origin, &span(0)));
    assert!(actual.is_forwarder(&first, &span(0)));
    assert!(actual.is_forwarder(&second, &span(0)));
    assert!(actual.is_forwarder(&second, &span(1)));
    for (source, rejected) in [(&decoy, 0), (&first, 1), (&first, 4), (&second, 4), (&second, 5)] {
        assert!(!actual.contains(source, &span(rejected)), "unseeded candidate has no authority");
    }
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
#[test]
fn multiple_receipt_relay_authority_is_qualified_and_single_hop() {
    let source = |path: &str, revision: &str| SourceIdentity::new(
        "project".into(), "payload".into(), path.into(), revision.into(),
    );
    let factory = source("factory.w", "1");
    let relay = source("relay.w", "1");
    let next = source("next.w", "1");
    let stale = source("factory.w", "0");
    let declaration = ByteSpan::new(1, 2);
    let roots = HashMap::from([(factory.clone(), HashMap::from([(declaration, MixedReceiptProof::Multiple)]))]);
    let users = HashMap::from([
        ((&factory, declaration), vec![(&relay, declaration)]),
        ((&relay, declaration), vec![(&next, declaration)]),
        ((&stale, declaration), vec![(&next, declaration)]),
    ]);
    let authority = MixedReceiptPlan { roots, users }.resolve();
    assert!(authority.is_multi(&factory, &declaration));
    assert!(authority.is_multi_forwarder(&relay, &declaration));
    assert!(!authority.contains(&relay, &declaration));
    assert!(!authority.is_multi(&next, &declaration));
    assert!(!authority.is_multi(&stale, &declaration));
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
    let mut targets = HashMap::new();
    let mut cohorts = Vec::new();
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
                let (results, current_errors, cohort) = derive_module_reference_cohort(
                    &module.source,
                    &module.items,
                    sites,
                    structs,
                    enums,
                    capture_call_points,
                    &mut targets,
                );
                if let Some(cohort) = cohort {
                    cohorts.push((updates.len(), cohort));
                }
                updates.push((index, results));
                errors.extend(current_errors);
            }
            Err(error) => errors.push(error.clone()),
        }
    }
    if errors.is_empty() && !targets.is_empty() {
        if let Some((source, span)) = (!call_sites.duplicate_sources.is_empty()).then(|| {
            modules.iter().find_map(|module| {
                let source = &module.source;
                if !call_sites.duplicate_sources.contains(source) { return None; }
                targets.get(source)?.iter().min_by_key(|span| (span.start, span.end)).map(|span| (source, span))
            })
        }).flatten() {
            errors.push(ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(source.clone(), *span),
                reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
            });
            return (updates, errors, inventory);
        }
        let grounds = cohorts.iter().filter_map(|(position, cohort)| {
            let (index, results) = &mut updates[*position];
            let module = &modules[*index];
            let source = &module.source;
            let sites = call_sites.modules[*index].as_ref().expect("successful mixed receipt cohort");
            let owners = cohort.grounds(source, &module.items, sites, results, &targets);
            (!owners.is_empty()).then(|| (source.clone(), owners))
        }).collect::<MixedReceiptRoots>();
        if !grounds.is_empty() {
            let (structs, enums) = inventory.as_ref().expect("mixed project cohorts have declared inventory");
            let mut edges = HashMap::new();
            for (position, cohort) in &cohorts {
                let (index, results) = &updates[*position];
                let module = &modules[*index];
                let sites = call_sites.modules[*index].as_ref().expect("successful mixed receipt cohort");
                cohort.index_forwarders(&module.source, &module.items, sites, results, &mut edges);
            }
            let authority = MixedReceiptPlan { roots: grounds, users: edges }.resolve();
            for (position, cohort) in cohorts {
                let (index, results) = &mut updates[position];
                let module = &modules[*index];
                let sites = call_sites.modules[*index].as_ref().expect("successful mixed receipt cohort");
                cohort.select(&module.source, &module.items, sites, results, &authority,
                    structs, enums, capture_call_points, &mut errors);
            }
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
enum ReferenceOutputOrigins {
    Explicit(Vec<ReferenceOrigin>),
    WholeGenericFormal { generic: String, parameter: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReferenceOutputContract<'a> {
    source: &'a SourceIdentity,
    declaration_span: ByteSpan,
    body: ReferenceCallableBody,
    output: usize,
    origins: ReferenceOutputOrigins,
    independent_static: bool,
}

impl ReferenceOutputContract<'_> {
    fn instantiate(
        &self, concrete: &ConcreteCallSelection, type_arguments: &[ScalarTypeArgument], unique: bool,
    ) -> Option<CallOriginTransfer<'_>> {
        match &self.origins {
            ReferenceOutputOrigins::Explicit(origins) => Some(CallOriginTransfer::Explicit(origins)),
            ReferenceOutputOrigins::WholeGenericFormal { generic, parameter } if unique => {
                let ty = match concrete {
                    ConcreteCallSelection::Overload(selected) => selected.substitutions.get(generic)?,
                    ConcreteCallSelection::Declaration => match type_arguments {
                        [argument] => &argument.ty,
                        _ => return None,
                    },
                };
                matches!(ty, ScalarType::CheckedReference { .. }).then_some(CallOriginTransfer::WholeFormal(*parameter))
            }
            ReferenceOutputOrigins::WholeGenericFormal { .. } => None,
        }
    }
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
            origins: ReferenceOutputOrigins::Explicit(origins
                .iter()
                .map(|id| {
                    independent = independent && result.aggregate.checked_result_origins.contains(id);
                    result.cfg.facts[id.0].clone()
                })
                .collect()),
            independent_static: false,
        });
    }
    if let Some(generic) = &result.whole_generic_formal {
        contracts.push(ReferenceOutputContract {
            source, declaration_span, body, output: 0,
            origins: ReferenceOutputOrigins::WholeGenericFormal { generic: generic.clone(), parameter: 0 },
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

fn contract_replay_reuse(
    proof: &CallablePointFacts,
    same_length: bool,
) -> (bool, Option<super::control::ForwardInitialGraph>) {
    let reuse_structure = proof.forward_initial_graph.is_some()
        && proof.pending_named_reborrow.is_none()
        && matches!(proof.scalar_descriptor_install, ScalarDescriptorCandidates::None)
        && matches!(proof.backing_descriptor_install, ScalarDescriptorCandidates::None);
    let topology = if reuse_structure && proof.scope_exit_bindings.is_some() && same_length {
        proof.forward_initial_graph
    } else {
        None
    };
    (reuse_structure, topology)
}

#[cfg(test)]
#[test]
fn prepared_contract_replay_reuses_only_eligible_topology() {
    use super::control::{DeferredReborrowRoot, ForwardInitialGraph, PendingNamedReborrow};

    let mut proof = CallablePointFacts::default();
    assert!(matches!(contract_replay_reuse(&proof, true), (false, None)));
    proof.scope_exit_bindings = Some(HashMap::new());
    assert!(matches!(contract_replay_reuse(&proof, true), (false, None)));
    proof.forward_initial_graph = Some(ForwardInitialGraph::Linear);
    assert!(matches!(contract_replay_reuse(&proof, true), (true, Some(ForwardInitialGraph::Linear))));
    assert!(matches!(contract_replay_reuse(&proof, false), (true, None)));
    proof.forward_initial_graph = Some(ForwardInitialGraph::Branched);
    assert!(matches!(contract_replay_reuse(&proof, true), (true, Some(ForwardInitialGraph::Branched))));
    proof.scope_exit_bindings = None;
    assert!(matches!(contract_replay_reuse(&proof, true), (true, None)));
    proof.scope_exit_bindings = Some(HashMap::new());
    proof.scalar_descriptor_install = ScalarDescriptorCandidates::Multiple;
    assert!(matches!(contract_replay_reuse(&proof, true), (false, None)));
    proof.scalar_descriptor_install = ScalarDescriptorCandidates::None;
    proof.backing_descriptor_install = ScalarDescriptorCandidates::Multiple;
    assert!(matches!(contract_replay_reuse(&proof, true), (false, None)));
    proof.backing_descriptor_install = ScalarDescriptorCandidates::None;
    proof.pending_named_reborrow = Some(PendingNamedReborrow {
        sites: Vec::new(),
        entry: CfgPointId(0),
        handle: ReferenceBindingId {
            source: SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into()),
            function_span: ByteSpan::new(0, 2),
            declaration_span: ByteSpan::new(0, 1),
            block_span: ByteSpan::new(0, 2),
            kind: ReferenceBindingKind::Declared,
        },
        root: DeferredReborrowRoot::Formal { parameter: 0 },
        origin: ReferenceOriginId(0),
    });
    assert!(matches!(contract_replay_reuse(&proof, true), (false, None)));
}

fn reset_contract_replay_points(
    live: &mut Vec<ReferenceFlowPoint>,
    pristine: &[ReferenceFlowPoint],
    reuse_structure: bool,
) {
    if live.len() != pristine.len() {
        *live = pristine.to_vec();
        return;
    }
    if !reuse_structure {
        for (live, pristine) in live.iter_mut().zip(pristine) {
            *live = pristine.clone();
        }
        return;
    }
    for (live, pristine) in live.iter_mut().zip(pristine) {
        if live.id == pristine.id && matches!((&live.kind, &pristine.kind),
            (CfgPointKind::Evaluate(_), CfgPointKind::Evaluate(_))
            | (CfgPointKind::Call { .. }, CfgPointKind::Call { .. })
            | (CfgPointKind::Branch { .. }, CfgPointKind::Branch { .. }))
        {
            live.possible_origins.clone_from(&pristine.possible_origins);
        } else {
            live.clone_from(pristine);
        }
    }
}

#[cfg(test)]
#[test]
fn contract_replay_reset_borrows_only_immutable_point_structure() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 10);
    let point = |kind| ReferenceFlowPoint {
        id: CfgPointId(0), kind, source_span: SourceSpan::new(source.clone(), span),
        scope: ReferenceScopeId(0), possible_origins: None, successors: vec![CfgPointId(1)],
    };
    let branch = point(CfgPointKind::Branch { condition: ScalarExpression::Boolean { value: true, span } });
    assert!(keep_forward_point_in_place(&branch.kind, true, 1));
    assert!(!keep_forward_point_in_place(&branch.kind, false, 1));
    assert!(!keep_forward_point_in_place(&branch.kind, true, 0));
    assert!(!keep_forward_point_in_place(&branch.kind, true, 2));
    let pristine = [branch.clone()];
    let mut live = vec![branch];
    live[0].possible_origins = Some(BTreeSet::from([ReferenceOriginId(0)]));
    let allocation = live[0].successors.as_ptr();
    reset_contract_replay_points(&mut live, &pristine, true);
    assert_eq!(live[0].successors.as_ptr(), allocation);
    assert!(live[0].possible_origins.is_none());
    live[0].kind = CfgPointKind::Evaluate(ScalarExpression::Boolean { value: false, span });
    assert!(keep_forward_point_in_place(&live[0].kind, true, 1));
    assert!(!keep_forward_point_in_place(&CfgPointKind::MoveThrough {
        place: ScalarPlace::Name { name: String::new(), span },
        ty: ScalarType::U64, pointer_point: CfgPointId(0), candidates: Vec::new(),
    }, true, 1));
    reset_contract_replay_points(&mut live, &pristine, true);
    assert!(matches!(&live[0].kind, CfgPointKind::Branch { condition: ScalarExpression::Boolean { value: true, .. } }));
    live.push(point(CfgPointKind::ScopeExit));
    reset_contract_replay_points(&mut live, &pristine, true);
    assert_eq!(live.len(), pristine.len());

    let evaluate = point(CfgPointKind::Evaluate(ScalarExpression::Boolean { value: false, span }));
    let pristine = [evaluate.clone()];
    let mut live = vec![evaluate];
    live[0].possible_origins = Some(BTreeSet::from([ReferenceOriginId(0)]));
    let allocation = live[0].successors.as_ptr();
    reset_contract_replay_points(&mut live, &pristine, true);
    assert_eq!(live[0].successors.as_ptr(), allocation);
    assert!(live[0].possible_origins.is_none());

    let call = point(CfgPointKind::Call {
        expression: ScalarExpression::Call {
            receiver: None, name: String::new(), receiver_span: None, name_span: span,
            type_arguments: Vec::new(), arguments: Vec::new(), overload_selection: None, span,
        },
        output: 0, argument_points: vec![CfgPointId(0)],
        target: ResolvedCallTarget::Core(CoreOperationId::IntExtend),
    });
    assert!(keep_forward_point_in_place(&call.kind, true, 1));
    let pristine = [call.clone()];
    let mut live = vec![call];
    let outer_allocation = live.as_ptr();
    let allocation = live[0].successors.as_ptr();
    reset_contract_replay_points(&mut live, &pristine, true);
    assert_eq!(live[0].successors.as_ptr(), allocation);
    reset_contract_replay_points(&mut live, &pristine, false);
    assert_eq!(live.as_ptr(), outer_allocation);
    assert_ne!(live[0].successors.as_ptr(), allocation);
}

fn transfer_function_output_contracts<'a>(
    result: &mut FunctionReferenceResult,
    previous: &ReferenceOutputContracts<'_>,
    contracts: &'a ReferenceOutputContracts<'_>,
    inputs: &mut HashMap<CfgPointId, CallOriginTransfer<'a>>,
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
        let current = contracts.find(source, *declaration_span, concrete, *output)
            .and_then(|(row, unique)| Some((row, unique, row.instantiate(concrete, type_arguments, unique)?)));
        if !changed {
            let prior = previous.find(source, *declaration_span, concrete, *output)
                .and_then(|(row, unique)| row.instantiate(concrete, type_arguments, unique));
            changed = current.map(|(_, _, transfer)| transfer) != prior;
        }
        if let Some((row, unique, transfer)) = current {
            inputs.insert(id, transfer);
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
        let (reuse_structure, topology) = contract_replay_reuse(
            &result.proof_points,
            result.cfg.points.len() == result.cfg.initial_points.len(),
        );
        reset_contract_replay_points(&mut result.cfg.points, &result.cfg.initial_points, reuse_structure);
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
        let mut transfer = ReferenceTransferCfg::new(&mut result.cfg, Some(&result.proof_points));
        transfer.forward_initial_graph = topology;
        if replay.module_calls.is_empty() {
            transfer.local_borrow = result.proof_points.local_borrow.as_deref();
        }
        let transferred = transfer_reference_facts(
            transfer,
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
        if !transferred.is_empty() {
            result.cfg.guarded_scalar_allocation = None;
            result.cfg.observed_runtime_allocations = None;
        }
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
    let mut errors = Vec::new();
    let mut updates = Vec::new();
    {
        let index = std::cell::OnceCell::new();
        let views: &[ModuleReferenceView<'_>] = modules;
        let templates = || index.get_or_init(|| ReferenceTemplateIndex::new(views));
        for module in views {
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
    }
    for (module, updates) in modules.iter_mut().zip(updates) {
        for ((_, result), update) in module.results.iter_mut().zip(updates) {
            update.apply(result);
        }
    }
    errors
}

fn derive_function_specialized_copy_policies<'a>(
    function: &ScalarFunction,
    result: &FunctionReferenceResult,
    templates: &impl Fn() -> &'a ReferenceTemplateIndex<'a>,
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
        let Some((declaration, declaration_result)) = templates()
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
                    local_borrow: None,
                    initial_points: &template_cfg.initial_points,
                    has_grouped_assignment_capture: template_result.proof_points.has_grouped_assignment_capture,
                    forward_initial_graph: None,
                    forward_graph_out: None,
                    scope_exit_bindings: template_result.proof_points.scope_exit_bindings.as_ref(),
                    pending_named_reborrow: template_result.proof_points.pending_named_reborrow.as_ref(),
                    resolve_pending_named_reborrow: false,
                    indexed_holders: None,
                    raw_address_exposures: &template_result.proof_points.raw_address_exposures,
                    points: &mut points,
                    facts: &mut facts,
                    provisional_origin_index: None,
                    allocations: &template_cfg.allocations,
                    checked_join_values: &template_result.proof_points.checked_join_values,
                    checked_block_values: &template_result.proof_points.checked_block_values,
                    pending_returned_extent: template_result.proof_points.pending_returned_extent,
                    widened_rebind_points: template_result.proof_points.widened_rebind_points.as_ref(),
                    array_length_owners: Some(&template_result.proof_points.array_length_owners).filter(|owners| !owners.is_empty()),
                    fallible_allocation_bindings: template_result.proof_points.fallible_allocation_bindings.as_ref(),
                    fallible_aliases: Some(&template_result.proof_points.fallible_aliases).filter(|aliases| !aliases.is_empty()),
                    guarded_allocation_edges: Some(&template_result.proof_points.guarded_allocation_edges),
                    guarded_indexed_stores: Some(&template_result.proof_points.guarded_indexed_stores),
                    guarded_runtime_moves: Some(&template_result.proof_points.guarded_runtime_moves).filter(|moves| !moves.is_empty()),
                    guarded_releases: Some(&template_result.proof_points.guarded_releases).filter(|releases| !releases.is_empty()),
                    checked_binding_initializers: &template_result.proof_points.checked_binding_initializers,
                    resolved_checked_call_outputs: &template_result.proof_points.resolved_checked_call_outputs,
                    checked_pointee_reads: &template_result.proof_points.checked_pointee_reads,
                    joined_reborrow_descendants: &template_result.proof_points.joined_reborrow_descendants,
                    joined_shared_aliases: &template_result.proof_points.joined_shared_aliases,
                    exclusive_scalar_write_moves: &template_result.proof_points.exclusive_scalar_write_moves,
                    shared_scalar_read_calls: &template_result.proof_points.shared_scalar_read_calls,
                    joined_shared_return_calls: &template_result.proof_points.joined_shared_return_calls,
                    joined_checked_releases: &template_result.proof_points.joined_checked_releases,
                    checked_named_scalar_stores: &template_result.proof_points.checked_named_scalar_stores,
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
    whole_generic_formal: Option<String>,
    proof_points: CallablePointFacts,
    aggregate: aggregate::AggregateFlowResult,
    arms: Vec<FunctionReferenceResult>,
}

impl FunctionReferenceResult {
    fn mixed_targets(&self) -> impl Iterator<Item = (&SourceIdentity, ByteSpan)> {
        self.aggregate.mixed_result_calls.iter().filter_map(move |point| {
            match &self.cfg.initial_points.get(point.0)?.kind {
                CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable {
                    source, declaration_span, concrete: ConcreteCallSelection::Declaration,
                }, .. } => Some((source, *declaration_span)),
                _ => None,
            }
        })
    }

    fn multi_targets(&self) -> impl Iterator<Item = (&SourceIdentity, ByteSpan)> {
        self.aggregate.multi_result_calls.iter().filter_map(|point| match &self.cfg.initial_points.get(point.0)?.kind {
            CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete: ConcreteCallSelection::Declaration }, .. } =>
                Some((source, *declaration_span)),
            _ => None,
        })
    }

    fn collect_mixed_targets(&self, targets: &mut MixedReceiptTargets) -> bool {
        let mut found = false;
        for (source, declaration) in self.mixed_targets().chain(self.multi_targets()) {
            if let Some(known) = targets.get_mut(source) {
                known.insert(declaration);
            } else {
                targets.insert(source.clone(), HashSet::from([declaration]));
            }
            found = true;
        }
        for arm in &self.arms {
            found |= arm.collect_mixed_targets(targets);
        }
        found
    }

    fn mixed_forwarded_callee(
        &self, function: &ScalarFunction, sites: &FunctionCallSites,
    ) -> Option<(&SourceIdentity, ByteSpan)> {
        let [ScalarBlockItem::LocalBinding(binding),
            ScalarBlockItem::Expression(ScalarExpression::Name { name: first_name, .. }),
            ScalarBlockItem::Expression(ScalarExpression::Name { name: second_name, .. })] =
            function.body.items.as_slice() else { return None };
        let [head, tail] = binding.receivers.as_slice() else { return None };
        if aggregate::mixed_owner_result_lanes(&head.ty, &tail.ty).is_none()
            || &head.name != first_name || &tail.name != second_name || first_name == second_name
            || binding.output_origin != ScalarBindingOutputOrigin::SingleExpression || binding.is_allocation
            || !matches!(binding.value, ScalarExpression::Call { ref arguments, .. }
                if arguments.iter().all(|argument| matches!(argument,
                    ScalarExpression::Name { .. } | ScalarExpression::Integer { .. }
                    | ScalarExpression::Boolean { .. } | ScalarExpression::Char { .. }
                    | ScalarExpression::Float { .. })))
        {
            return None;
        }
        let ScalarType::Callable { parameters, outputs } = &function.signature else { return None };
        let [first, second] = outputs.outputs.as_slice() else { return None };
        if first.ty != head.ty || second.ty != tail.ty || self.aggregate.mixed_result_calls.len() != 1
            || !parameters.iter().all(aggregate::ordinary_value)
        {
            return None;
        }
        let call = self.aggregate.mixed_result_calls.first()?;
        let point = self.cfg.initial_points.get(call.0)?;
        let CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable {
            source: actual, declaration_span, concrete: ConcreteCallSelection::Declaration,
        }, .. } = &point.kind else { return None };
        let Some(OccurrenceNode::Expression(ordinal)) = sites.structure.edges.get(&(
            OccurrenceParent::Block(sites.structure.root), OccurrenceEdge::BindingRoot { item: 0, output: 0 },
        )) else { return None };
        let row = sites.checked_call_signatures.get(ordinal)?;
        let [call_head, call_tail] = row.outputs.outputs.as_slice() else { return None };
        (call_head.ty == head.ty && call_tail.ty == tail.ty
            && row.parameters.iter().all(aggregate::ordinary_value)).then_some((actual, *declaration_span))
    }

    fn multi_forwarded_callee(
        &self, function: &ScalarFunction, sites: &FunctionCallSites,
    ) -> Option<(&SourceIdentity, ByteSpan)> {
        let root = sites.structure.blocks.get(sites.structure.root.0)?;
        let (first, rest) = root.items.split_first()?;
        let BlockItemKind::Binding(region) = &first.kind else { return None };
        let ScalarBlockItem::LocalBinding(binding) = function.body.items.first()? else { return None };
        let ScalarExpression::Call { arguments, .. } = &binding.value else { return None };
        let receivers = region.receivers();
        let outputs = &sites.boundary.root_outputs;
        if outputs.len() < 2 || rest.len() != outputs.len() || receivers.len() != outputs.len()
            || binding.is_allocation || binding.output_origin != ScalarBindingOutputOrigin::SingleExpression
            || !sites.boundary.parameters.iter().all(|parameter| aggregate::ordinary_value(&parameter.ty))
            || !arguments.iter().all(|argument| matches!(argument, ScalarExpression::Name { .. }
                | ScalarExpression::Integer { .. } | ScalarExpression::Boolean { .. }
                | ScalarExpression::Char { .. } | ScalarExpression::Float { .. }))
            || self.aggregate.multi_result_calls.len() != 1
        { return None; }
        let OccurrenceNode::Expression(ordinal) = *sites.structure.edges.get(&(
            OccurrenceParent::Block(sites.structure.root), OccurrenceEdge::BindingRoot { item: 0, output: 0 },
        ))? else { return None };
        if sites.declared_unsafe_calls.contains(&ordinal.0) { return None; }
        let signature = sites.checked_call_signatures.get(&ordinal)?;
        if !signature.parameters.iter().all(aggregate::ordinary_value)
            || !signature.outputs.outputs.iter().map(|output| &output.ty).eq(outputs.iter().map(|output| &output.ty))
        { return None; }
        for (lane, ((item, receiver), ty)) in rest.iter().zip(receivers).zip(outputs).enumerate() {
            let BlockItemKind::FinalOutput(output) = &item.kind else { return None };
            if output.output != lane || receiver.position != lane || receiver.ty != ty.ty
                || !matches!(ty.ty, ScalarType::Struct(_))
                || !matches!(sites.prepared_plain_name(output.expression), Some(PlainNameIdentity::Lexical(id))
                    if id.declaration_span == receiver.span && id.block_span == root.span
                        && id.kind == ReferenceBindingKind::Declared)
            { return None; }
        }
        let point = self.cfg.initial_points.get(self.aggregate.multi_result_calls.first()?.0)?;
        if point.source_span.range != sites.occurrences.get(ordinal.0)?.span { return None; }
        match &point.kind {
            CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable {
                source, declaration_span, concrete: ConcreteCallSelection::Declaration }, .. } => Some((source, *declaration_span)),
            _ => None,
        }
    }

    fn observe_selected_mixed_receipts(
        &mut self, function: &ScalarFunction, sites: &FunctionCallSites, module_names: &HashSet<String>,
        structs: &[ScalarStruct], enums: &[ScalarEnum], source: &SourceIdentity,
        effects: &HashMap<ByteSpan, SharedScalarReadEffect>, owners: &MixedReceiptAuthority,
        forwards: bool, capture_call_points: bool, errors: &mut Vec<ReferenceAnalysisError>,
    ) {
        for ((result, function), sites) in self.arms.iter_mut().zip(&function.overload_arms).zip(&sites.arms) {
            result.observe_selected_mixed_receipts(function, sites, module_names, structs, enums, source, effects, owners,
                false, capture_call_points, errors);
        }
        if !self.mixed_targets().any(|(target, declaration)| owners.contains(target, &declaration))
            && !self.multi_targets().any(|(target, declaration)| owners.is_multi(target, &declaration))
        {
            return;
        }
        let mut builder = ReferenceFlowBuilder::new(source, function, module_names, sites, structs, enums);
        builder.capture_call_points = capture_call_points;
        builder.mixed_owners = Some(owners);
        builder.aggregate_facts.multi_forwarder = owners.is_multi_forwarder(source, &function.name_span);
        builder.shared_scalar_read_effects = Some(effects);
        let (cfg, emitted_errors, proof_points, mut aggregate, checked_contract_returns) =
            build_function_reference_cfg(builder, function, true);
        if forwards && aggregate.mixed_result_bindings.len() == 1 {
            aggregate.mixed_forward = aggregate.mixed_result_bindings.first().copied();
        }
        errors.extend(emitted_errors);
        self.cfg = cfg;
        self.proof_points = proof_points;
        self.aggregate = aggregate;
        self.checked_contract_returns = checked_contract_returns;
    }

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
            if self.aggregate.has_observations()
                || !self.aggregate.ordinary_fallible_call_candidates.is_empty()
                || !self.aggregate.ordinary_fallible_expressions.is_empty()
                || self.aggregate.guarded_array_producer_choice.is_some()
                || !self.aggregate.ordinary_array_receivers.is_empty()
            {
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
        None,
        false,
        &mut errors,
    );
    result.install(function);
    errors
}

fn seal_guarded_scalar_allocations(
    cfg: &ScalarReferenceCfg, points: &CallablePointFacts,
) -> Option<Box<[GuardedScalarAllocation]>> {
    let bindings = points.fallible_allocation_bindings.as_ref()?;
    if bindings.is_empty() { return None; }
    // Index each qualified owner once; neither guards nor alias accesses
    // scan the set of live fallible allocations.
    let mut owners: Vec<(&ReferenceBindingId, Vec<GuardedScalarBranch>, Vec<usize>)> = Vec::new();
    let mut owner_indices = HashMap::with_capacity(bindings.len());
    for (branch, edge) in &points.guarded_allocation_edges {
        let point = cfg.points.get(branch.0)?;
        let [then, otherwise] = point.successors.as_slice() else { return None };
        if !matches!(point.kind, CfgPointKind::Branch { .. })
            || !((*then == edge.present && *otherwise == edge.absent)
                || (*then == edge.absent && *otherwise == edge.present))
        {
            return None;
        }
        bindings.get(&edge.binding)?;
        let index = *owner_indices.entry(&edge.binding).or_insert_with(|| {
            let index = owners.len();
            owners.push((&edge.binding, Vec::new(), Vec::new()));
            index
        });
        owners[index].1.push(GuardedScalarBranch {
            condition: edge.condition.0, branch: *branch, present_on_then: *then == edge.present,
        });
    }
    if owners.len() != bindings.len() {
        return None;
    }
    for (ordinal, owner) in &points.fallible_alias_accesses {
        owners[*owner_indices.get(owner)?].2.push(ordinal.0);
    }
    let mut moves_by_owner = HashMap::<&ReferenceBindingId, Vec<GuardedRuntimeMove>>::new();
    for (read, receipt) in &points.guarded_runtime_moves {
        let source = bindings.get(&receipt.source)?;
        if !aggregate::same_runtime_array_type(&source.ty, &receipt.target_ty) { return None; }
        let before = cfg.points.get(read.0)?;
        let moved = cfg.points.get(receipt.moved.0)?;
        let [installed] = moved.successors.as_slice() else { return None; };
        let installed = cfg.points.get(installed.0)?;
        let exit = points.guarded_allocation_scopes.get(&receipt.scope.0)?.exit;
        if before.scope != receipt.scope || moved.scope != receipt.scope || installed.scope != receipt.scope
            || !matches!(&before.kind, CfgPointKind::Read { binding } if binding == &receipt.source)
            || before.successors.as_slice() != [moved.id]
            || !matches!(&moved.kind, CfgPointKind::Move { place, owner, ty }
                if owner == &receipt.source && place.binding == receipt.source && place.projections.is_empty()
                    && aggregate::same_runtime_array_type(ty, &receipt.target_ty))
            || !match (&installed.kind, receipt.replacement) {
                (CfgPointKind::Bind { binding }, None) => binding == &receipt.target,
                (CfgPointKind::Assign { target, rhs_point, .. }, Some(_)) =>
                    target.binding == receipt.target && target.projections.is_empty() && rhs_point == read,
                _ => false,
            }
            || !points.scope_exit_bindings.as_ref()?.get(&exit)?.contains(&receipt.target)
        { return None; }
        moves_by_owner.entry(&receipt.source).or_default().push(receipt.clone());
    }
    let mut releases_by_owner = HashMap::<&ReferenceBindingId, Vec<ByteSpan>>::new();
    for (point, binding) in &points.guarded_releases {
        let release = cfg.points.get(point.0)?;
        let CfgPointKind::Release { target: ReleaseTarget::Checked { candidates, .. } } = &release.kind else { return None; };
        let [candidate] = candidates.as_slice() else { return None; };
        if &candidate.place.binding != binding || !candidate.place.projections.is_empty()
            || !bindings.get(binding)?.has_runtime_extent()
        { return None; }
        releases_by_owner.entry(binding).or_default().push(release.source_span.range);
    }
    owners.into_iter().map(|(binding, mut guards, mut accesses)| {
        let declared = bindings.get(binding)?;
        let scope = points.guarded_allocation_scopes.get(&declared.scope.0)?;
        let (entry, exit) = (cfg.points.get(scope.entry.0)?, cfg.points.get(scope.exit.0)?);
        if scope.scope != declared.scope || entry.scope != declared.scope || exit.scope != declared.scope
            || scope.entry >= scope.exit || entry.source_span.range != binding.block_span
            || exit.source_span.range != binding.block_span
            || !matches!(entry.kind, CfgPointKind::ScopeEnter)
            || !matches!(exit.kind, CfgPointKind::ScopeExit)
            || !points.scope_exit_bindings.as_ref()?.get(&scope.exit)?.contains(binding)
        { return None; }
        guards.sort_unstable_by_key(|guard| guard.condition);
        if guards.windows(2).any(|pair| pair[0].condition == pair[1].condition) {
            return None;
        }
        accesses.sort_unstable();
        accesses.dedup();
        Some(GuardedScalarAllocation {
            binding: binding.clone(), ty: declared.ty.clone(), runtime_literal_length: declared.runtime_literal_length,
            runtime_returned: declared.runtime_returned,
            guards: guards.into_boxed_slice(), alias_accesses: accesses.into_boxed_slice(),
            runtime_moves: moves_by_owner.remove(binding).unwrap_or_default().into_boxed_slice(),
            releases: releases_by_owner.remove(binding).unwrap_or_default().into_boxed_slice(),
        })
    }).collect()
}

// Record only a prepared exact unary template identity. Generic output
// types have no abstract loan, and no origin is invented for the template.
fn prepared_whole_generic_formal(calls: &FunctionCallSites, cfg: &ScalarReferenceCfg) -> Option<String> {
    let [generic] = calls.boundary.generics.as_slice() else { return None };
    let [parameter] = calls.boundary.parameters.as_slice() else { return None };
    let [output] = calls.boundary.root_outputs.as_slice() else { return None };
    let generic_type = |ty: &ScalarType| matches!(ty, ScalarType::Named { name, .. } if name == &generic.name);
    let [item] = calls.structure.blocks.get(calls.structure.root.0)?.items.as_slice() else { return None };
    let BlockItemKind::FinalOutput(final_value) = &item.kind else { return None };
    let PlainNameIdentity::Lexical(formal) = calls.prepared_plain_name(final_value.expression)? else { return None };
    if parameter.ordinal != 0 || output.output != 0 || final_value.output != 0
        || !generic_type(&parameter.ty) || !generic_type(&output.ty)
        || !calls.structure.checked_types.expressions.get(&final_value.expression).is_some_and(generic_type)
        || formal.declaration_span != parameter.span || formal.kind != ReferenceBindingKind::Declared
        || !cfg.allocations.is_empty()
    { return None; }
    let points = &cfg.initial_points;
    let mut saw_read = None;
    let mut saw_move = false;
    let mut saw_return = false;
    for (index, point) in points.iter().enumerate() {
        if points.get(index + 1).map_or(!point.successors.is_empty(), |next| point.successors.as_slice() != [next.id]) {
            return None;
        }
        match &point.kind {
            CfgPointKind::Entry | CfgPointKind::ScopeEnter | CfgPointKind::ScopeExit | CfgPointKind::Return => {},
            CfgPointKind::Bind { binding } if binding == formal => {},
            CfgPointKind::Read { binding } if binding == formal && saw_read.is_none() => saw_read = Some(point.id),
            CfgPointKind::DeferredMove { place, owner, ty } if saw_read.is_some() && !saw_move
                && owner == formal && &place.binding == formal && place.projections.is_empty() && generic_type(ty) =>
                    saw_move = true,
            CfgPointKind::ReturnOutput { output: 0, value_point, checked: false }
                if saw_move && !saw_return && Some(*value_point) == saw_read => saw_return = true,
            _ => return None,
        }
    }
    saw_return.then(|| generic.name.clone())
}

fn derive_function_reference_result(
    source: &SourceIdentity,
    function: &ScalarFunction,
    module_names: &HashSet<String>,
    call_sites: &FunctionCallSites,
    structs: &[ScalarStruct],
    enums: &[ScalarEnum],
    effects: Option<&HashMap<ByteSpan, SharedScalarReadEffect>>,
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
            effects,
            capture_call_points,
            errors,
        ));
    }
    let mut builder =
        ReferenceFlowBuilder::new(source, function, module_names, call_sites, structs, enums);
    builder.capture_call_points = capture_call_points;
    builder.shared_scalar_read_effects = effects;
    let (cfg, current_errors, proof_points, aggregate, checked_contract_returns) =
        build_function_reference_cfg(builder, function, errors.len() == entry_errors);
    let whole_generic_formal = current_errors.is_empty().then(|| prepared_whole_generic_formal(call_sites, &cfg)).flatten();
    errors.extend(current_errors);
    FunctionReferenceResult {
        checked_contract_returns,
        whole_generic_formal,
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
    let (cfg, errors, _, _, _) =
        build_function_reference_cfg(builder, function, prior_children_succeeded);
    (cfg, errors)
}

fn parameter_binding_identity(
    source: &SourceIdentity,
    function_span: ByteSpan,
    block_span: ByteSpan,
    parameter: &ParameterSlot,
) -> ReferenceBindingId {
    ReferenceBindingId {
        source: source.clone(),
        function_span,
        declaration_span: parameter.span,
        block_span,
        kind: ReferenceBindingKind::Declared,
    }
}

fn fresh_recommit_leaf_extent(
    source: &SourceIdentity, function: &ScalarFunction, calls: &FunctionCallSites,
    cfg: &ScalarReferenceCfg, allocations: &BTreeMap<ReferenceBindingId, CfgPointId>,
) -> Option<aggregate::RecommittedArrayExtent> {
    let ScalarType::Callable { parameters, outputs } = &function.signature else { return None };
    if !aggregate::neutral_array_signature(parameters, outputs)
        || !function.generic_parameters.is_empty() || !function.overload_arms.is_empty()
        || !calls.call_occurrences.is_empty() || !calls.structure.module_storage_uses.is_empty()
        || allocations.len() != 1
    {
        return None;
    }
    let [ScalarBlockItem::LocalBinding(declaration), ..] = function.body.items.as_slice() else {
        return None;
    };
    let [receiver] = declaration.receivers.as_slice() else { return None };
    if !declaration.is_allocation || declaration.fallible_assignment.is_some()
        || !aggregate::same_runtime_array_type(&receiver.ty, &outputs.outputs[0].ty)
    {
        return None;
    }
    let (owner, &bind) = allocations.iter().next()?;
    if owner.declaration_span != receiver.name_span {
        return None;
    }
    let BlockItemKind::Binding(region) = &calls.structure.blocks[calls.structure.root.0].items.first()?.kind else {
        return None;
    };
    let BindingInitialization::Allocation { lengths } = &region.initialization else { return None };
    let [length] = lengths.as_slice() else { return None };
    if length.position != 0 || calls.structure.checked_types.expressions.get(&length.expression)
        != Some(&ScalarType::U64)
    {
        return None;
    }
    let extent = match receiver.allocation_length.as_ref()? {
        ScalarExpression::Integer { value, .. } => aggregate::RecommittedArrayExtent::Literal(
            u64::try_from(value.clone()).ok()?,
        ),
        ScalarExpression::Name { .. } => {
            let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(length.expression)? else {
                return None;
            };
            let formal = calls.boundary.parameters.iter().find(|slot| slot.ty == ScalarType::U64
                && binding.declaration_span == slot.span
                && binding == &parameter_binding_identity(source, function.span, function.body.span, slot))?;
            aggregate::RecommittedArrayExtent::Formal(formal.ordinal)
        }
        _ => return None,
    };

    let mut returned = false;
    let mut moved = None;
    for point in &cfg.points[bind.0 + 1..] {
        match &point.kind {
            CfgPointKind::Assign { target, .. } if target.binding == *owner && target.projections.is_empty() => return None,
            CfgPointKind::Borrow { .. } | CfgPointKind::Release { .. } | CfgPointKind::Invalidate { .. }
            | CfgPointKind::AssignThrough { .. } | CfgPointKind::MoveThrough { .. }
            | CfgPointKind::DeferredMoveThrough { .. }
            | CfgPointKind::Evaluate(ScalarExpression::RawAddress { .. } | ScalarExpression::CheckedAddress { .. }) => return None,
            CfgPointKind::Move { owner: moved_owner, .. } if moved_owner == owner => {
                if moved.replace(point.id).is_some() { return None }
            }
            CfgPointKind::ReturnOutput { output: 0, value_point, .. } => {
                let read = cfg.points.get(value_point.0)?;
                if returned || !matches!(&read.kind, CfgPointKind::Read { binding } if binding == owner)
                    || moved.is_none_or(|moved| read.successors.as_slice() != [moved])
                {
                    return None;
                }
                returned = true;
            }
            _ => {}
        }
    }
    returned.then_some(extent)
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
    Vec<CheckedContractReturn>,
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
            parameter_binding_identity(&builder.source, function.span, function.body.span, slot),
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
            .record(builder.names[name].clone(), ty.clone(), None);
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
    let has_flow = prior_children_succeeded && errors.is_empty() && !builder.cfg.points.is_empty();
    if has_flow {
        builder.seal_local_borrow();
    }
    let mut provisional_origin_index = (has_flow
        && (builder.proof_points.pending_named_reborrow.is_some()
            || builder.has_joined_root_reborrow)).then_some(builder.origin_index);
    let mut aggregate = std::mem::take(&mut builder.aggregate_result);
    if let ScalarDescriptorCandidates::One(_, marker) = &builder.proof_points.backing_descriptor_install {
        aggregate.passive_descriptor_read = passive_descriptor_read(
            &builder.cfg, marker, &builder.aggregate_facts, call_sites,
            &builder.proof_points, builder.structs).map(Box::new);
    }
    if has_flow {
        builder.cfg.initial_points = builder.cfg.points.clone();
        if let Some((Some(moved), Some(call))) = builder.formal_forward_shape {
            let terminal = builder.cfg.points.last().map(|point| &point.kind);
            let exact_args = match &builder.cfg.points[call.0].kind {
                CfgPointKind::Call { argument_points, .. } if argument_points.len() == 1 => true,
                CfgPointKind::Call { argument_points, .. } => (|| {
                    let points = &builder.cfg.points;
                    let first = points.get(argument_points.first()?.0)?;
                    let (&formal, holder) = builder.aggregate_facts.formals.iter().next()?;
                    let ty = &call_sites.boundary.parameters.get(formal)?.ty;
                    let (ordinal, moves) = builder.aggregate_facts.input_moves.get(&call)?;
                    let signature = call_sites.checked_call_signatures.get(ordinal)?;
                    let (prepared, slot, exact_move) = aggregate::exact_owning_call_arguments(
                        points, first, points[call.0].scope, call_sites, *ordinal,
                        &signature.parameters, holder, ty,
                    )?;
                    Some(moves.len() == 1 && prepared.id == call && exact_move == moved
                        && moves.get(&slot) == Some(&aggregate::ArgumentProvenance::WholeMove(moved)))
                })().unwrap_or(false),
                _ => false,
            };
            if exact_args && (matches!(terminal, Some(CfgPointKind::Return))
                || (call_sites.boundary.root_outputs.is_empty()
                    && matches!(terminal, Some(CfgPointKind::ScopeExit))))
            {
                builder.aggregate_facts.formal_forward_shape = Some((moved, call));
            }
        }
        builder.aggregate_facts.multi_formal_forward_shape =
            aggregate::exact_multi_formal_forward(&builder.cfg.points, call_sites, &builder.aggregate_facts);
        let flow = builder.aggregate_facts.observation_mode().map(|mode| (
            &builder.aggregate_facts,
            mode,
            call_sites,
            &mut aggregate,
        ));
        let mut forward_initial_graph = None;
        let mut provisional_cfg = ReferenceTransferCfg::new(&mut builder.cfg, Some(&builder.proof_points));
        provisional_cfg.local_borrow = builder.proof_points.local_borrow.as_deref();
        provisional_cfg.forward_graph_out = Some(&mut forward_initial_graph);
        provisional_cfg.provisional_origin_index = provisional_origin_index.as_mut();
        provisional_cfg.resolve_pending_named_reborrow = true;
        let provisional = transfer_reference_facts(
            provisional_cfg,
            &BTreeMap::new(),
            &HashMap::new(),
            flow,
            None,
        );
        drop(provisional_origin_index);
        builder.proof_points.forward_initial_graph = forward_initial_graph;
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
                let differs_in_scope = |point| deferred_local_reborrow_differs_in_scope(pending, point, &builder.cfg.initial_points);
                let first = &builder.cfg.initial_points[pending.sites[0].point.0];
                if pending.sites.len() > 1 || is_pending_shared_reborrow(first) || differs_in_scope(first) {
                    errors.extend(provisional.iter().filter(|error| matches!(error,
                        ReferenceAnalysisError::ConflictingPlace { source_span, origins } if pending.sites.iter().any(|site| {
                            let solved = &builder.cfg.points[site.point.0];
                            let CfgPointKind::Borrow { loan, .. } = &solved.kind else { return false };
                            solved.source_span == *source_span
                                || ((site.parent.is_some() || loan.mode == ScalarReferenceMutability::Shared || differs_in_scope(solved))
                                        && origins.contains(&solved.source_span)
                                            && builder.proof_points.checked_pointee_reads.iter()
                                                .chain(&builder.proof_points.checked_named_scalar_stores)
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
        if builder.has_joined_root_reborrow {
            errors.extend(provisional.iter().cloned());
        } else {
            errors.extend(provisional.iter().filter(|error| matches!(error,
                ReferenceAnalysisError::MissingContext {
                    reason: ReferenceAnalysisErrorReason::UnprovedArrayLength
                        | ReferenceAnalysisErrorReason::UnprovedWidenedArrayTransfer, ..
                }
            )).cloned());
        }
        if builder.proof_points.fallible_allocation_bindings.is_some() {
            errors.extend(provisional.iter().filter(|error| matches!(error,
                ReferenceAnalysisError::MissingOrigin {
                    reason: ReferenceAnalysisErrorReason::UnavailableFallibleAllocation
                        | ReferenceAnalysisErrorReason::UnsupportedFallibleAllocationAccess
                        | ReferenceAnalysisErrorReason::UnsupportedFallibleWholeAggregateRead, ..
                }
            )).cloned());
        }
        if !builder.has_joined_root_reborrow && !provisional.is_empty() {
            // Preserve ordinary loan-conflict provenance for guarded indexed
            // borrows; an overlapping live loan is not a failed null guard.
            let sites = builder.proof_points.fallible_allocation_bindings.as_ref();
            let is_guarded_array = |owner| sites.and_then(|sites| sites.get(owner))
                .is_some_and(|site| site.has_indexed_extent());
            let moves = &builder.proof_points.guarded_runtime_moves;
            if builder.proof_points.fallible_aliases.values().any(is_guarded_array) || !moves.is_empty() {
                let origins = builder.cfg.initial_points.iter().filter_map(|point|
                    matches!(&point.kind, CfgPointKind::Borrow { place, .. } if is_guarded_array(&place.binding))
                        .then_some(&point.source_span)).chain(moves.values().filter_map(|receipt|
                            builder.cfg.initial_points.get(receipt.moved.0).map(|point| &point.source_span)))
                    .collect::<HashSet<_>>();
                let mut diagnosed = errors.iter().filter_map(|error|
                    matches!(error, ReferenceAnalysisError::ConflictingPlace { .. })
                        .then_some(error.source_span().clone())).collect::<HashSet<_>>();
                for error in &provisional {
                    if let ReferenceAnalysisError::ConflictingPlace { source_span, origins: related } = error {
                        if related.iter().any(|origin| origins.contains(origin)) && diagnosed.insert(source_span.clone()) {
                            errors.push(error.clone());
                        }
                    }
                }
            }
        }
        if provisional.is_empty() {
            if errors.is_empty() {
                builder.cfg.guarded_scalar_allocation = seal_guarded_scalar_allocations(&builder.cfg, &builder.proof_points);
            }
            for &(end, _) in &builder.aggregate_facts.checked_producer_arms {
                aggregate.checked_producer_backing.entry(end).or_insert(aggregate::CheckedProducerBacking::Unknown);
            }
            aggregate.checked_producer_sources = std::mem::take(&mut builder.aggregate_facts.checked_producer_arms);
            if errors.is_empty() {
                builder.aggregate_facts.seal_mixed_branch_return(
                    &builder.cfg, &builder.cfg.initial_points[builder.root_return_points.clone()], &mut aggregate,
                );
            }
        } else {
            aggregate.checked_producer_backing.clear();
        }
        if errors.is_empty() && builder.cfg.guarded_scalar_allocation.is_none() {
            if let Some(bindings) = &builder.proof_points.fallible_allocation_bindings {
                if let Some(binding) = bindings.keys().min() {
                    errors.push(ReferenceAnalysisError::MissingOrigin {
                        source_span: SourceSpan::new(builder.source.clone(), binding.declaration_span),
                        binding_id: binding.clone(),
                        reason: ReferenceAnalysisErrorReason::MissingGuardedAllocationProof,
                    });
                }
            }
        }
    }
    if has_flow && prior_children_succeeded && errors.is_empty() {
        aggregate.fresh_recommit_leaf = fresh_recommit_leaf_extent(
            &builder.source, function, call_sites, &builder.cfg, &builder.aggregate_facts.allocations,
        );
    }
    aggregate.returning_leaf = errors.is_empty() && prior_children_succeeded && builder.returning_leaf_prefix
        && matches!(builder.cfg.points.last().map(|point| &point.kind), Some(CfgPointKind::Return));
    let root_return_points = if has_flow {
        &builder.cfg.initial_points[builder.root_return_points]
    } else {
        &[]
    };
    let checked_contract_returns = CheckedContractReturn::collect(&function.signature, root_return_points);
    if builder.independent_checked_query {
        aggregate.checked_successor_candidate = aggregate.formal_forward.map(|(_, point, _, _)| point);
        if !root_return_points.is_empty()
            && (!builder.aggregate_facts.raw_formals.is_empty()
                || !builder.aggregate_facts.read_fields.is_empty())
        {
            let returned = root_return_points.iter().rev().find_map(|point| {
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
    if has_flow && errors.is_empty() && prior_children_succeeded
        && !builder.proof_points.array_length_owners.is_empty()
    {
        let mut declarations = builder.proof_points.array_length_owners.iter()
            .map(|owner| (owner.declaration_span.start, owner.declaration_span.end))
            .collect::<Vec<_>>();
        declarations.sort_unstable();
        builder.cfg.observed_runtime_allocations = Some(declarations.into_boxed_slice());
    }
    if !errors.is_empty() || !prior_children_succeeded {
        aggregate.array_returns.clear();
        aggregate.array_rebinds.clear();
        aggregate.local_array_failure_arms.clear();
        aggregate.checked_producer_backing.clear();
    }
    aggregate.borrowed_extent_observer = builder.aggregate_facts.borrowed_extent_observer;
    aggregate.borrowed_extent_call = builder.aggregate_facts.borrowed_extent_call;
    aggregate.borrowed_extent_forward = builder.aggregate_facts.borrowed_extent_forward;
    aggregate.array_formal_extent_required = !builder.aggregate_facts.array_formal_extent_borrows.is_empty();
    aggregate.literal_array_extent_required = builder.aggregate_facts.literal_array_extent_output
        .map(|(_, occurrence)| occurrence);
    aggregate.literal_array_index_required = builder.aggregate_facts.literal_array_index_output;
    aggregate.array_calls = builder.aggregate_facts.array_calls;
    aggregate.checked_result_origins = builder.checked_result_origins;
    if (2..=aggregate::MAX_LOCAL_CHOICE_LEAVES).contains(&builder.aggregate_facts.calls.len()) {
        let structure = &call_sites.structure;
        if matches!(structure.blocks[structure.root.0].items.first().map(|item| &item.kind),
            Some(BlockItemKind::Binding(binding))
                if matches!(&binding.initialization, BindingInitialization::Outputs(init)
                    if matches!(call_sites.conditionals.get(&init.primary), Some(ConditionalRegion::Value { .. }))))
        {
            aggregate.local_choice_reader_calls = std::mem::take(&mut builder.aggregate_facts.receipt_loan_read_calls);
        }
    }
    aggregate.allocation_slots = builder.aggregate_facts.allocation_slots;
    // Install the canonical owning-call census before sealing local results.
    aggregate.calls = builder.aggregate_facts.calls;
    if errors.is_empty() {
        if builder.aggregate_facts.multi_forwarder {
            aggregate.sealed_multi_forward = aggregate.multi_forward_result(&builder.cfg, call_sites);
        } else {
            aggregate.sealed_multi_result = aggregate.multi_local_result(&builder.cfg, call_sites);
        }
    }
    aggregate.formals = builder.aggregate_facts.formals;
    aggregate.read_formals = builder.aggregate_facts.raw_formals.into_keys().collect();
    aggregate.read_formal_fields = builder.aggregate_facts.read_fields;
    (builder.cfg, errors, builder.proof_points, aggregate, checked_contract_returns)
}

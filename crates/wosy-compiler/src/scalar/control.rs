use std::collections::{HashMap, HashSet};

use super::reference::{BlockOrdinal, OccurrenceMetadata, OccurrenceOrdinal, OccurrenceStructure};
use super::{
    ConcreteCallSelection, CoreOperationId, ScalarBinding, ScalarEnumId, ScalarItem,
    ScalarNamespaceBinding, ScalarOutputSequence, ScalarOverloadSelection, ScalarStructFieldId,
    ScalarType,
};
use wosy_syntax::{ByteSpan, SourceIdentity, SourceSpan};

mod initializer_call_nodes;
pub(crate) use super::reference::{
    DirectDestinationIdentity, NamedPlaceRootIdentity, PlainNameIdentity,
};
pub(crate) use initializer_call_nodes::{
    BoundInitializerMember, CallableBodyCallNodes, InitializerCallNodes, InitializerFacts,
    InitializerFactsError,
};

#[derive(Default)]
pub(crate) struct CallablePointFacts {
    pub(super) evaluations: HashMap<OccurrenceOrdinal, super::CfgPointId>,
    pub(super) installs: HashMap<OccurrenceOrdinal, FrozenInstall>,
    pub(super) definitions: HashMap<(BlockOrdinal, usize, usize), BindingDefinition>,
    pub(super) formals: HashMap<usize, super::ReferenceBindingId>,
    pub(super) scalar_descriptor_install: ScalarDescriptorCandidates,
    pub(super) backing_descriptor_install: ScalarDescriptorCandidates,
    pub(super) has_nonlexical_install: bool,
}

#[derive(Default)]
pub(super) enum ScalarDescriptorCandidates {
    #[default]
    None,
    One(OccurrenceOrdinal, Box<ScalarDescriptorInstall>),
    Multiple,
}

#[derive(Clone, Debug)]
pub(crate) struct ScalarDescriptorInstall {
    pub(crate) validate_backing: bool,
    pub(crate) frozen: FrozenInstall,
    pub(crate) capture: super::CfgPointId,
    pub(crate) binding: super::ReferenceBindingId,
    pub(crate) call: OccurrenceOrdinal,
    pub(crate) output: usize,
    pub(crate) structure: super::ScalarStructId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrozenInstall {
    pub(crate) rhs: super::CfgPointId,
    pub(crate) install: super::CfgPointId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BindingDefinition {
    Allocation {
        bind: super::CfgPointId,
    },
    Initialized {
        bind: super::CfgPointId,
        value: super::CfgPointId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModuleInitializationRegion {
    pub(crate) source: SourceIdentity,
    pub(crate) steps: Vec<ModuleInitializationStep>,
    pub(super) detail: ModuleInitializationDetail,
}

impl ModuleInitializationRegion {
    #[cfg(test)]
    pub(crate) fn remove_first_member_fact(&mut self, remove_type: bool) -> (ByteSpan, usize) {
        use super::reference::InitializerMemberRequirement;
        let ModuleInitializationDetail::Occurrences(topology) = &mut self.detail else {
            panic!("prepared initializer topology");
        };
        let (index, span) = topology
            .occurrences
            .iter()
            .enumerate()
            .find_map(|(index, metadata)| match metadata.initializer_member {
                InitializerMemberRequirement::Token(span) => Some((index, span)),
                InitializerMemberRequirement::NotRequired => None,
            })
            .expect("initializer member");
        let ordinal = OccurrenceOrdinal(index);
        if remove_type {
            assert!(topology
                .structure
                .checked_types
                .expressions
                .remove(&ordinal)
                .is_some());
        } else {
            assert!(topology.identities.members.remove(&ordinal).is_some());
        }
        (span, index)
    }

    #[cfg(test)]
    pub(crate) fn remove_only_call_target(&mut self) {
        let ModuleInitializationDetail::Occurrences(topology) = &mut self.detail else {
            panic!("prepared initializer topology");
        };
        let [ordinal] = topology.calls.as_slice() else {
            panic!("exactly one initializer call");
        };
        assert!(topology.call_targets.remove(ordinal).is_some());
    }

    pub(crate) fn bind_call_nodes<'a>(
        &'a self,
        items: &'a [ScalarItem],
    ) -> Result<InitializerCallNodes<'a>, String> {
        InitializerCallNodes::bind(self, items)
    }

    pub(super) fn call_target(
        &self,
        ordinal: OccurrenceOrdinal,
    ) -> Option<&ModuleInitializationCallTarget> {
        match &self.detail {
            ModuleInitializationDetail::DispatchOnly => None,
            ModuleInitializationDetail::Occurrences(topology) => {
                topology.call_targets.get(&ordinal)
            }
        }
    }

    pub(crate) fn legacy_reservation_targets(
        &self,
    ) -> Result<impl Iterator<Item = (usize, bool, &ModuleInitializationCallTarget)>, String> {
        let ModuleInitializationDetail::Occurrences(topology) = &self.detail else {
            return Err(format!(
                "internal error: missing initializer call topology for {:?}",
                self.source
            ));
        };
        Ok(topology
            .legacy_reservations
            .iter()
            .map(|&(item, ordinal, unqualified)| {
                (item, unqualified, &topology.call_targets[&ordinal])
            }))
    }

    pub(crate) fn call_targets(
        &self,
    ) -> Result<impl Iterator<Item = Result<&ModuleInitializationCallTarget, String>>, String> {
        let ModuleInitializationDetail::Occurrences(topology) = &self.detail else {
            return Err(format!(
                "internal error: missing initializer call topology for {:?}",
                self.source,
            ));
        };
        Ok(topology.calls.iter().map(|&ordinal| {
            self.call_target(ordinal)
                .ok_or_else(|| topology.missing_call_target(&self.source, ordinal))
        }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ModuleInitializationDetail {
    DispatchOnly,
    Occurrences(Box<ModuleInitializationTopology>),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ModuleInitializationTopology {
    pub(super) structure: OccurrenceStructure,
    pub(super) occurrences: Vec<OccurrenceMetadata>,
    pub(super) calls: Vec<OccurrenceOrdinal>,
    pub(super) call_targets: HashMap<OccurrenceOrdinal, ModuleInitializationCallTarget>,
    pub(super) legacy_reservations: Vec<(usize, OccurrenceOrdinal, bool)>,
    pub(super) checked_call_signatures: HashMap<OccurrenceOrdinal, CheckedCallSignature>,
    pub(super) identities: InitializationIdentities,
    pub(super) callable_escapes: HashSet<SourceSpan>,
    pub(super) assignments: HashMap<(BlockOrdinal, usize), AssignmentGroup>,
    pub(super) conditionals: HashMap<OccurrenceOrdinal, ConditionalRegion>,
    #[cfg(any(test, feature = "internal-checker"))]
    pub(super) initializer_predicates: HashMap<OccurrenceOrdinal, super::checker_directive::InitializerPredicate>,
    pub(super) short_circuits: HashMap<OccurrenceOrdinal, ShortCircuitRegion>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct InitializationIdentities {
    pub(super) names: HashMap<OccurrenceOrdinal, InitializationName>,
    pub(super) roots: HashMap<OccurrenceOrdinal, InitializationStorage>,
    pub(super) destinations: HashMap<OccurrenceOrdinal, InitializationDestination>,
    pub(super) members: HashMap<OccurrenceOrdinal, InitializationMember>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InitializationMember {
    Field {
        receiver: InitializationStorage,
        receiver_name: String,
        field: ScalarStructFieldId,
        through_raw_pointer: bool,
    },
    Module {
        qualifier: ByteSpan,
        declaration: SourceSpan,
        name: String,
    },
    Enum {
        enumeration: ScalarEnumId,
        tag: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InitializationLocal {
    pub(crate) declaration_span: ByteSpan,
    pub(crate) block_span: ByteSpan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InitializationStorage {
    Lexical(InitializationLocal),
    Module(ByteSpan),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InitializationName {
    Storage(InitializationStorage),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InitializationDestination {
    Existing(InitializationStorage),
    Introduce(InitializationLocal),
    IntroduceModule(ByteSpan),
}

impl ModuleInitializationTopology {
    pub(in crate::scalar) fn missing_call_target(&self, source: &SourceIdentity, ordinal: OccurrenceOrdinal) -> String {
        let span = self.occurrences[ordinal.0].span;
        format!(
            "internal error: missing initializer call target in {source:?} at {}..{} (occurrence {})",
            span.start, span.end, ordinal.0,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ModuleInitializationCallTarget {
    Core(CoreOperationId),
    Direct {
        declaration: SourceSpan,
        concrete: ConcreteCallSelection,
    },
    Generic {
        declaration: SourceSpan,
        type_arguments: Vec<ScalarType>,
    },
    Extern(SourceSpan),
    CallableBinding(SourceSpan),
    CallableField {
        receiver: SourceSpan,
        field: ScalarStructFieldId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CheckedCallSignature {
    pub(super) parameters: Vec<ScalarType>,
    pub(super) outputs: ScalarOutputSequence,
    pub(super) overload: Option<ScalarOverloadSelection>,
    pub(super) overload_origin: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct CheckedOccurrenceTypes {
    pub(super) expressions: HashMap<OccurrenceOrdinal, ScalarType>,
    pub(super) places: HashMap<OccurrenceOrdinal, ScalarType>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FoldedOccurrence {
    SignedIntegerMagnitude,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SyntheticOccurrence {
    QueryAllocationPlaceholder,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModuleInitializationStep {
    pub(crate) item: usize,
    pub(crate) kind: ModuleInitializationStepKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ModuleInitializationStepKind {
    NamespaceDispatch {
        binding: String,
        binding_span: ByteSpan,
        target: NamespaceTarget,
    },
    BindingInitialization,
    ExecutableItem,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NamespaceTarget {
    Bound(SourceIdentity),
    Unbound,
}

pub(crate) fn module_initialization_region(
    source: &SourceIdentity,
    items: &[ScalarItem],
    namespace_bindings: &[ScalarNamespaceBinding],
) -> ModuleInitializationRegion {
    let mut namespace_targets = HashMap::new();
    for binding in namespace_bindings {
        namespace_targets
            .entry(binding.binding.as_str())
            .or_insert(&binding.target);
    }
    let steps = items
        .iter()
        .enumerate()
        .filter_map(|(item, declaration)| {
            let kind = match declaration {
                ScalarItem::Namespace(namespace) => {
                    ModuleInitializationStepKind::NamespaceDispatch {
                        binding: namespace.binding.clone(),
                        binding_span: namespace.binding_span,
                        target: match namespace_targets.get(namespace.binding.as_str()) {
                            Some(target) => NamespaceTarget::Bound((*target).clone()),
                            None => NamespaceTarget::Unbound,
                        },
                    }
                }
                ScalarItem::Binding(_) => ModuleInitializationStepKind::BindingInitialization,
                ScalarItem::Executable(_) => ModuleInitializationStepKind::ExecutableItem,
                ScalarItem::Function(_) | ScalarItem::Extern(_) => return None,
            };
            Some(ModuleInitializationStep { item, kind })
        })
        .collect();
    ModuleInitializationRegion {
        source: source.clone(),
        steps,
        detail: ModuleInitializationDetail::DispatchOnly,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AssignmentEvaluation {
    pub(super) position: usize,
    pub(super) expression: OccurrenceOrdinal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AssignmentProjection {
    pub(super) producer: AssignmentEvaluation,
    pub(super) output: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum AssignmentReplacement {
    Resolved(ScalarType),
    Missing,
    StructuralOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AssignmentSlot {
    pub(super) position: usize,
    pub(super) destination: OccurrenceOrdinal,
    pub(super) target_span: ByteSpan,
    pub(super) replacement: AssignmentReplacement,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AssignmentGroup {
    pub(super) span: ByteSpan,
    evaluations: Vec<AssignmentEvaluation>,
    projections: Vec<AssignmentProjection>,
    slots: Vec<AssignmentSlot>,
    next: BlockPosition,
}

impl AssignmentGroup {
    pub(super) fn new(
        span: ByteSpan,
        evaluations: Vec<AssignmentEvaluation>,
        projections: Vec<AssignmentProjection>,
        slots: Vec<AssignmentSlot>,
        next: BlockPosition,
    ) -> Self {
        Self {
            span,
            evaluations,
            projections,
            slots,
            next,
        }
    }

    pub(super) fn rhs_evaluations(&self) -> std::slice::Iter<'_, AssignmentEvaluation> {
        self.evaluations.iter()
    }

    pub(super) fn projected_outputs(&self) -> std::slice::Iter<'_, AssignmentProjection> {
        self.projections.iter()
    }

    pub(super) fn destinations(&self) -> std::slice::Iter<'_, AssignmentSlot> {
        self.slots.iter()
    }

    pub(super) fn previous_observations(&self) -> std::slice::Iter<'_, AssignmentSlot> {
        self.slots.iter()
    }

    pub(super) fn installs(&self) -> std::slice::Iter<'_, AssignmentSlot> {
        self.slots.iter()
    }

    pub(super) fn continuation(&self) -> BlockPosition {
        self.next
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FinalOutputSlot {
    pub(super) item: usize,
    pub(super) output: usize,
    pub(super) expression: OccurrenceOrdinal,
    pub(super) span: ByteSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConditionalOutputProjection {
    pub(super) types: Vec<ScalarType>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BindingOutput {
    Evaluate {
        position: usize,
        expression: OccurrenceOrdinal,
    },
    ProjectPrimary {
        output: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BindingReceiverSlot {
    pub(super) position: usize,
    pub(super) name: String,
    pub(super) span: ByteSpan,
    pub(super) ty: ScalarType,
    pub(super) contextual_storage: Option<super::ReferenceBindingId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BindingAllocationLengthSlot {
    pub(super) position: usize,
    pub(super) expression: OccurrenceOrdinal,
    pub(super) ty: ScalarType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BindingOutputs {
    pub(super) primary: OccurrenceOrdinal,
    pub(super) secondary: Vec<BindingOutput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BindingInitialization {
    Outputs(BindingOutputs),
    Allocation {
        lengths: Vec<BindingAllocationLengthSlot>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BindingRegion {
    pub(super) initialization: BindingInitialization,
    receivers: Vec<BindingReceiverSlot>,
}

impl BindingRegion {
    pub(super) fn new(initialization: BindingInitialization, binding: &ScalarBinding) -> Self {
        let mut receivers = Vec::with_capacity(binding.receivers.len().max(1));
        receivers.push(BindingReceiverSlot {
            position: 0,
            name: binding.name.clone(),
            span: binding.name_span,
            ty: binding.declared_type.clone(),
            contextual_storage: binding.receivers.first().and_then(|receiver| receiver.contextual_storage.clone()),
        });
        receivers.extend(binding.receivers.iter().enumerate().skip(1).map(
            |(position, receiver)| BindingReceiverSlot {
                position,
                name: receiver.name.clone(),
                span: receiver.name_span,
                ty: receiver.ty.clone(),
                contextual_storage: receiver.contextual_storage.clone(),
            },
        ));
        Self {
            initialization,
            receivers,
        }
    }

    pub(super) fn receivers(&self) -> &[BindingReceiverSlot] {
        &self.receivers
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BlockItemKind {
    Binding(BindingRegion),
    Evaluation { expression: OccurrenceOrdinal },
    FinalOutput(FinalOutputSlot),
    Assignment { group: BlockPosition },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BlockItem {
    pub(super) position: usize,
    pub(super) kind: BlockItemKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BlockRegion {
    pub(super) span: ByteSpan,
    pub(super) items: Vec<BlockItem>,
    projected_outputs: Vec<FinalOutputSlot>,
}

impl BlockRegion {
    pub(super) fn new(span: ByteSpan, items: Vec<BlockItem>) -> Self {
        Self {
            span,
            items,
            projected_outputs: Vec::new(),
        }
    }

    pub(super) fn project_tail_outputs(&mut self, primary: FinalOutputSlot, arity: usize) {
        debug_assert_eq!(primary.output, 0);
        debug_assert!(self.projected_outputs.is_empty());
        self.projected_outputs
            .extend((1..arity).map(|output| FinalOutputSlot { output, ..primary }));
    }

    pub(super) fn final_outputs(&self) -> impl Iterator<Item = &FinalOutputSlot> {
        self.items
            .iter()
            .filter_map(|item| match &item.kind {
                BlockItemKind::FinalOutput(slot) => Some(slot),
                _ => None,
            })
            .chain(&self.projected_outputs)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ConditionalRegion {
    Value {
        span: ByteSpan,
        condition: OccurrenceOrdinal,
        then_block: BlockOrdinal,
        else_block: BlockOrdinal,
        resume: OccurrenceOrdinal,
    },
    Unit {
        span: ByteSpan,
        condition: OccurrenceOrdinal,
        then_block: BlockOrdinal,
        fallthrough: OccurrenceOrdinal,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BlockPosition {
    pub(super) block: BlockOrdinal,
    pub(super) item: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ShortCircuitRegion {
    pub(super) span: ByteSpan,
    pub(super) left: OccurrenceOrdinal,
    pub(super) right: OccurrenceOrdinal,
    pub(super) evaluate_right_when: bool,
    pub(super) bypass: bool,
    pub(super) join: OccurrenceOrdinal,
}

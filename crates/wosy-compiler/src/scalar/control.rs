use std::collections::HashMap;

use super::reference::{BlockOrdinal, OccurrenceMetadata, OccurrenceOrdinal, OccurrenceStructure};
use super::{ScalarBinding, ScalarItem, ScalarNamespaceBinding, ScalarOutputSequence, ScalarType};
use wosy_syntax::{ByteSpan, SourceIdentity};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModuleInitializationRegion {
    pub(crate) source: SourceIdentity,
    pub(crate) steps: Vec<ModuleInitializationStep>,
    pub(super) detail: ModuleInitializationDetail,
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
    pub(super) checked_call_signatures: HashMap<OccurrenceOrdinal, CheckedCallSignature>,
    pub(super) assignments: HashMap<(BlockOrdinal, usize), AssignmentGroup>,
    pub(super) conditionals: HashMap<OccurrenceOrdinal, ConditionalRegion>,
    pub(super) loops: HashMap<(BlockOrdinal, usize), WhileRegion>,
    pub(super) short_circuits: HashMap<OccurrenceOrdinal, ShortCircuitRegion>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CheckedCallSignature {
    pub(super) parameters: Vec<ScalarType>,
    pub(super) outputs: ScalarOutputSequence,
    pub(super) overload_arm: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct CheckedOccurrenceTypes {
    pub(super) expressions: HashMap<OccurrenceOrdinal, ScalarType>,
    pub(super) places: HashMap<OccurrenceOrdinal, ScalarType>,
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
        });
        receivers.extend(binding.receivers.iter().enumerate().skip(1).map(
            |(position, receiver)| BindingReceiverSlot {
                position,
                name: receiver.name.clone(),
                span: receiver.name_span,
                ty: receiver.ty.clone(),
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
    While { region: BlockPosition },
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
}

impl BlockRegion {
    pub(super) fn new(span: ByteSpan, items: Vec<BlockItem>) -> Self {
        Self { span, items }
    }

    pub(super) fn final_outputs(&self) -> impl Iterator<Item = &FinalOutputSlot> {
        self.items.iter().filter_map(|item| match &item.kind {
            BlockItemKind::FinalOutput(slot) => Some(slot),
            _ => None,
        })
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
pub(super) struct WhileRegion {
    pub(super) span: ByteSpan,
    pub(super) test: OccurrenceOrdinal,
    pub(super) body: BlockOrdinal,
    pub(super) on_false: BlockPosition,
}

impl WhileRegion {
    pub(super) fn body_to_test(&self) -> (BlockOrdinal, OccurrenceOrdinal) {
        (self.body, self.test)
    }
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

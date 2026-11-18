use std::collections::HashMap;

use super::reference::{BlockOrdinal, OccurrenceOrdinal};
use super::{ScalarItem, ScalarNamespaceBinding, ScalarType};
use wosy_syntax::{ByteSpan, SourceIdentity};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModuleInitializationRegion {
    pub(crate) source: SourceIdentity,
    pub(crate) steps: Vec<ModuleInitializationStep>,
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
pub(super) struct BindingRegion {
    pub(super) primary: OccurrenceOrdinal,
    pub(super) secondary: Vec<BindingOutput>,
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

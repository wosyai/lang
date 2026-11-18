use super::reference::{BlockOrdinal, OccurrenceOrdinal};
use wosy_syntax::ByteSpan;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AssignmentSlot {
    pub(super) position: usize,
    pub(super) destination: OccurrenceOrdinal,
    pub(super) target_span: ByteSpan,
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

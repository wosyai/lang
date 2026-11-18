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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AssignmentContinuation {
    Unit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AssignmentGroup {
    pub(super) span: ByteSpan,
    evaluations: Vec<AssignmentEvaluation>,
    projections: Vec<AssignmentProjection>,
    slots: Vec<AssignmentSlot>,
}

impl AssignmentGroup {
    pub(super) fn new(
        span: ByteSpan,
        evaluations: Vec<AssignmentEvaluation>,
        projections: Vec<AssignmentProjection>,
        slots: Vec<AssignmentSlot>,
    ) -> Self {
        Self {
            span,
            evaluations,
            projections,
            slots,
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

    pub(super) fn continuation(&self) -> AssignmentContinuation {
        AssignmentContinuation::Unit
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
pub(super) struct ValueArm {
    pub(super) block: BlockOrdinal,
    pub(super) outputs: Vec<FinalOutputSlot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ConditionalRegion {
    Value {
        span: ByteSpan,
        condition: OccurrenceOrdinal,
        then_arm: ValueArm,
        else_arm: ValueArm,
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

use super::super::reference::source_epoch::SourceEventId;
use super::*;

#[derive(Debug)]
pub(super) struct CaptureInputPair<'a> {
    pub(super) declaration: &'a closure::ClosureCapture,
    pub(super) snapshot: &'a CallableCaptureInput,
}

#[derive(Debug)]
pub(super) struct CaptureInputTuple<'a> {
    pairs: Vec<CaptureInputPair<'a>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CaptureInputTupleError {
    Arity {
        event: SourceEventId,
        source_span: SourceSpan,
        declared: usize,
        supplied: usize,
    },
    Binding {
        event: SourceEventId,
        source_span: SourceSpan,
        ordinal: usize,
        expected: ReferenceBindingId,
        actual: ReferenceBindingId,
    },
    DuplicateDeclaration {
        event: SourceEventId,
        source_span: SourceSpan,
        ordinal: usize,
        binding: ReferenceBindingId,
    },
}

impl<'a> CaptureInputTuple<'a> {
    pub(super) fn new(
        owner: &ProducerCfgOwnerId,
        capture: CfgPointId,
        capture_span: &SourceSpan,
        declarations: &'a [closure::ClosureCapture],
        path: &'a [CallableCaptureInput],
    ) -> Result<Self, CaptureInputTupleError> {
        let event = SourceEventId {
            owner: owner.clone(),
            point: capture,
        };
        if declarations.len() != path.len() {
            return Err(CaptureInputTupleError::Arity {
                event,
                source_span: capture_span.clone(),
                declared: declarations.len(),
                supplied: path.len(),
            });
        }
        let mut pairs = Vec::new();
        for (ordinal, declaration) in declarations.iter().enumerate() {
            if declarations[..ordinal]
                .iter()
                .any(|previous| previous.binding == declaration.binding)
            {
                return Err(CaptureInputTupleError::DuplicateDeclaration {
                    event,
                    source_span: capture_span.clone(),
                    ordinal,
                    binding: declaration.binding.clone(),
                });
            }
            let snapshot = &path[ordinal];
            if declaration.binding != snapshot.binding {
                return Err(CaptureInputTupleError::Binding {
                    event,
                    source_span: capture_span.clone(),
                    ordinal,
                    expected: declaration.binding.clone(),
                    actual: snapshot.binding.clone(),
                });
            }
            pairs.push(CaptureInputPair {
                declaration,
                snapshot,
            });
        }
        Ok(Self { pairs })
    }

    pub(super) fn pairs(&self) -> &[CaptureInputPair<'a>] {
        &self.pairs
    }
}

#[cfg(test)]
#[path = "capture_input_tuple_tests.rs"]
mod tests;

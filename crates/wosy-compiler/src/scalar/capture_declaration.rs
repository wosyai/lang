use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceParameterDeclaration<'a> {
    pub ordinal: usize,
    pub name: &'a String,
    pub binding: ReferenceBindingId,
    pub ty: &'a ScalarType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCaptureDeclaration<'a> {
    pub event: SourceEventId,
    pub literal_owner: ProducerCfgOwnerId,
    pub literal_span: &'a SourceSpan,
    pub signature: &'a ScalarType,
    pub outputs: &'a ScalarOutputSequence,
    pub body: &'a ScalarBlock,
    pub captures: &'a [closure::ClosureCapture],
    pub parameters: Vec<SourceParameterDeclaration<'a>>,
}

impl<'a> SourceCaptureDeclaration<'a> {
    pub fn new(
        parent_owner: &ProducerCfgOwnerId,
        source_span: &'a SourceSpan,
        point: CfgPointId,
        literal: ByteSpan,
        environment_literal: &SourceSpan,
        signature: &'a ScalarType,
        captures: &'a [closure::ClosureCapture],
        parameters: &'a [String],
        parameter_spans: &[ByteSpan],
        body: &'a ScalarBlock,
    ) -> Result<Self, SourceEpochError> {
        let event = SourceEventId {
            owner: parent_owner.clone(),
            point,
        };
        if literal != source_span.range || environment_literal != source_span {
            return Err(SourceEpochError::MalformedCaptureLiteral {
                event,
                source_span: source_span.clone(),
                literal,
                environment_literal: environment_literal.clone(),
            });
        }
        let ScalarType::Callable {
            outputs,
            parameters: parameter_types,
        } = signature
        else {
            return Err(SourceEpochError::InvalidCaptureSignature {
                event,
                signature: signature.clone(),
            });
        };
        if parameters.len() != parameter_spans.len() || parameters.len() != parameter_types.len() {
            return Err(SourceEpochError::CaptureParameterArity {
                event,
                signature: signature.clone(),
                names: parameters.len(),
                spans: parameter_spans.len(),
                types: parameter_types.len(),
            });
        }
        for (ordinal, capture) in captures.iter().enumerate() {
            if captures[..ordinal]
                .iter()
                .any(|previous| previous.binding == capture.binding)
            {
                return Err(SourceEpochError::DuplicateCaptureBinding {
                    event,
                    binding: capture.binding.clone(),
                });
            }
        }
        let literal_owner = ProducerCfgOwnerId::Literal {
            enclosing: Box::new(parent_owner.clone()),
            capture_point: point,
            literal: source_span.clone(),
        };
        let parameters = parameters
            .iter()
            .zip(parameter_spans)
            .zip(parameter_types)
            .enumerate()
            .map(|(ordinal, ((name, span), ty))| SourceParameterDeclaration {
                ordinal,
                name,
                ty,
                binding: ReferenceBindingId {
                    source: source_span.source.clone(),
                    function_span: literal,
                    declaration_span: *span,
                    block_span: body.span,
                    kind: ReferenceBindingKind::Declared,
                },
            })
            .collect();
        Ok(Self {
            event,
            literal_owner,
            literal_span: source_span,
            signature,
            outputs,
            body,
            captures,
            parameters,
        })
    }
}

#[cfg(test)]
#[path = "capture_declaration_tests.rs"]
mod tests;

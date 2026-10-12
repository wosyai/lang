use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConditionalOutputBlockCorrespondence {
    pub selected: ScalarBlockOccurrence,
    pub source: ScalarBlockOccurrence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConditionalOutputGroup {
    pub expression: ScalarExpression,
    pub blocks: Vec<ConditionalOutputBlockCorrespondence>,
}

fn invalid(source: &SourceIdentity, span: ByteSpan) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: SourceSpan::new(source.clone(), span),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }
}

pub(crate) fn group_conditional_final_outputs(
    source: &SourceIdentity,
    block: &ScalarBlock,
) -> Result<Option<ConditionalOutputGroup>, ReferenceAnalysisError> {
    if block.final_output_values.is_empty() {
        return Ok(None);
    }
    let first = &block.final_output_values[0];
    let ScalarExpression::If {
        condition,
        then_branch,
        else_branch,
        span,
    } = &first.value
    else {
        return Ok(None);
    };
    let same_source = block.final_output_values.iter().all(|output| {
        matches!(&output.value, ScalarExpression::If { span: candidate, .. } if candidate == span)
    });
    let projected = block.final_output_values.iter().any(|output| {
        matches!(&output.value, ScalarExpression::If { then_branch, else_branch, .. }
            if then_branch.occurrence.output_projection != block.occurrence.output_projection
                || else_branch.occurrence.output_projection != block.occurrence.output_projection)
    });
    if !projected {
        return Ok(None);
    }
    let error = || invalid(source, *span);
    if !same_source || block.final_output_values.len() < 2 {
        return Err(error());
    }
    verify_block(source, block).map_err(|_| error())?;
    let mut qualifier = SelectionInverse {
        source,
        mode: OccurrenceMode::Source,
        blocks: Vec::new(),
    };
    qualifier.expression(&mut condition.as_ref().clone())?;
    let mut inverse = SelectionInverse {
        source,
        mode: OccurrenceMode::Selected(0),
        blocks: Vec::new(),
    };
    let mut then_outputs = Vec::new();
    let mut else_outputs = Vec::new();
    let mut whole_then = None;
    let mut whole_else = None;
    for (position, output) in block.final_output_values.iter().enumerate() {
        let ScalarExpression::If {
            condition: candidate_condition,
            then_branch: candidate_then,
            else_branch: candidate_else,
            span: candidate_span,
        } = &output.value
        else {
            return Err(error());
        };
        if output.position != position
            || output.span != *span
            || candidate_span != span
            || candidate_condition != condition
            || candidate_then.span != then_branch.span
            || candidate_else.span != else_branch.span
        {
            return Err(error());
        }
        inverse.mode = OccurrenceMode::Selected(position);
        let mut normalized_then = candidate_then.clone();
        let mut normalized_else = candidate_else.clone();
        inverse.block(&mut normalized_then)?;
        inverse.block(&mut normalized_else)?;
        if normalized_then.occurrence.output_projection != block.occurrence.output_projection
            || normalized_else.occurrence.output_projection != block.occurrence.output_projection
        {
            return Err(error());
        }
        append_branch(
            source,
            position,
            normalized_then,
            &mut whole_then,
            &mut then_outputs,
        )?;
        append_branch(
            source,
            position,
            normalized_else,
            &mut whole_else,
            &mut else_outputs,
        )?;
    }
    let mut then_branch = whole_then.expect("verified projected then arm");
    let mut else_branch = whole_else.expect("verified projected else arm");
    compose_branch(&mut then_branch, then_outputs);
    compose_branch(&mut else_branch, else_outputs);
    Ok(Some(ConditionalOutputGroup {
        expression: ScalarExpression::If {
            condition: condition.clone(),
            then_branch,
            else_branch,
            span: *span,
        },
        blocks: inverse.blocks,
    }))
}

fn verify_block(
    source: &SourceIdentity,
    block: &ScalarBlock,
) -> Result<(), ReferenceAnalysisError> {
    let error = || invalid(source, block.span);
    if block.occurrence.source_span != SourceSpan::new(source.clone(), block.span)
        || block.items.len() != block.terminated_items.len()
        || block.items.len() < block.final_output_values.len()
    {
        return Err(error());
    }
    let expressions = block
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if expressions != block.expressions {
        return Err(error());
    }
    let start = block.items.len() - block.final_output_values.len();
    for (index, output) in block.final_output_values.iter().enumerate() {
        if block.items[start + index] != ScalarBlockItem::Expression(output.value.clone())
            || block.terminated_items[start + index]
        {
            return Err(error());
        }
    }
    Ok(())
}

fn append_branch(
    source: &SourceIdentity,
    position: usize,
    branch: ScalarBlock,
    whole: &mut Option<ScalarBlock>,
    outputs: &mut Vec<ScalarOutputValue>,
) -> Result<(), ReferenceAnalysisError> {
    let [output] = branch.final_output_values.as_slice() else {
        return Err(invalid(source, branch.span));
    };
    if output.position != position {
        return Err(invalid(source, branch.span));
    }
    if let Some(first) = whole {
        let first_start = first.items.len() - 1;
        let start = branch.items.len() - 1;
        if first.occurrence != branch.occurrence
            || first.span != branch.span
            || first.unsafe_context != branch.unsafe_context
            || first.items[..first_start] != branch.items[..start]
            || first.terminated_items[..first_start] != branch.terminated_items[..start]
        {
            return Err(invalid(source, branch.span));
        }
    }
    outputs.push(output.clone());
    if whole.is_none() {
        *whole = Some(branch);
    }
    Ok(())
}

fn compose_branch(branch: &mut ScalarBlock, outputs: Vec<ScalarOutputValue>) {
    let start = branch.items.len() - 1;
    branch.items.truncate(start);
    branch.terminated_items.truncate(start);
    for output in &outputs {
        branch
            .items
            .push(ScalarBlockItem::Expression(output.value.clone()));
        branch.terminated_items.push(false);
    }
    branch.final_output_values = outputs;
    branch.expressions = branch
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
}

struct SelectionInverse<'a> {
    source: &'a SourceIdentity,
    mode: OccurrenceMode,
    blocks: Vec<ConditionalOutputBlockCorrespondence>,
}

enum OccurrenceMode {
    Source,
    Selected(usize),
}

impl SelectionInverse<'_> {
    fn occurrence(
        &self,
        source_span: &SourceSpan,
        span: ByteSpan,
        projection: &mut Vec<usize>,
    ) -> Result<(), ReferenceAnalysisError> {
        if *source_span != SourceSpan::new(self.source.clone(), span) {
            return Err(invalid(self.source, span));
        }
        if let OccurrenceMode::Selected(position) = self.mode {
            if projection.first() != Some(&position) {
                return Err(invalid(self.source, span));
            }
            projection.remove(0);
        }
        Ok(())
    }

    fn block(&mut self, block: &mut ScalarBlock) -> Result<(), ReferenceAnalysisError> {
        verify_block(self.source, block)?;
        let selected = block.occurrence.as_ref().clone();
        self.occurrence(
            &block.occurrence.source_span,
            block.span,
            &mut block.occurrence.output_projection,
        )?;
        let correspondence = ConditionalOutputBlockCorrespondence {
            selected,
            source: block.occurrence.as_ref().clone(),
        };
        if matches!(self.mode, OccurrenceMode::Selected(_)) {
            if let Some(existing) = self
                .blocks
                .iter()
                .find(|entry| entry.selected == correspondence.selected)
            {
                if existing.source != correspondence.source {
                    return Err(invalid(self.source, block.span));
                }
            } else {
                self.blocks.push(correspondence);
            }
        }
        for item in &mut block.items {
            match item {
                ScalarBlockItem::LocalBinding(binding) => {
                    self.occurrence(
                        &binding.occurrence.source_span,
                        binding.span,
                        &mut binding.occurrence.output_projection,
                    )?;
                    self.expression(&mut binding.value)?;
                    for output in &mut binding.output_values {
                        self.expression(&mut output.value)?;
                    }
                    for receiver in &mut binding.receivers {
                        if let Some(length) = &mut receiver.allocation_length {
                            self.expression(length)?;
                        }
                    }
                }
                ScalarBlockItem::Expression(expression) => self.expression(expression)?,
                ScalarBlockItem::Assignment(assignment) => self.assignment(assignment)?,
                ScalarBlockItem::While(value) => {
                    self.occurrence(
                        &value.occurrence.source_span,
                        value.span,
                        &mut value.occurrence.output_projection,
                    )?;
                    self.expression(&mut value.condition)?;
                    self.block(&mut value.body)?;
                }
            }
        }
        for expression in &mut block.expressions {
            self.expression(expression)?;
        }
        for output in &mut block.final_output_values {
            self.expression(&mut output.value)?;
        }
        verify_block(self.source, block)
    }

    fn assignment(
        &mut self,
        assignment: &mut ScalarAssignment,
    ) -> Result<(), ReferenceAnalysisError> {
        self.occurrence(
            &assignment.occurrence.source_span,
            assignment.span,
            &mut assignment.occurrence.output_projection,
        )?;
        for target in &mut assignment.targets {
            self.place(&mut target.place)?;
        }
        for value in &mut assignment.values {
            self.expression(value)?;
        }
        Ok(())
    }

    fn place(&mut self, place: &mut ScalarPlace) -> Result<(), ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { .. } => {}
            ScalarPlace::Field { base, .. } => self.place(base)?,
            ScalarPlace::Dereference { pointer, .. } => self.expression(pointer)?,
            ScalarPlace::Index { base, index, .. } => {
                self.place(base)?;
                self.expression(index)?;
            }
        }
        Ok(())
    }

    fn expression(
        &mut self,
        expression: &mut ScalarExpression,
    ) -> Result<(), ReferenceAnalysisError> {
        match expression {
            ScalarExpression::Assignment(assignment) => self.assignment(assignment)?,
            ScalarExpression::FunctionLiteral { body, .. } | ScalarExpression::Block(body) => {
                self.block(body)?
            }
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(condition)?;
                self.block(then_branch)?;
                self.block(else_branch)?;
            }
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                self.expression(condition)?;
                self.block(then_branch)?;
            }
            ScalarExpression::Binary { left, right, .. } => {
                self.expression(left)?;
                self.expression(right)?;
            }
            ScalarExpression::Unary { operand, .. } => self.expression(operand)?,
            ScalarExpression::Call {
                arguments,
                occurrence,
                span,
                ..
            } => {
                self.occurrence(
                    &occurrence.source_span,
                    *span,
                    &mut occurrence.output_projection,
                )?;
                for argument in arguments {
                    self.expression(argument)?;
                }
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for element in elements {
                    self.expression(element)?;
                }
            }
            ScalarExpression::StructLiteral { fields, .. } => {
                for field in fields {
                    self.expression(&mut field.value)?;
                }
            }
            ScalarExpression::Dereference {
                place,
                occurrence,
                span,
            }
            | ScalarExpression::IndexedRead {
                place,
                occurrence,
                span,
            } => {
                self.occurrence(
                    &occurrence.source_span,
                    *span,
                    &mut occurrence.output_projection,
                )?;
                self.place(place)?;
            }
            ScalarExpression::RawAddress { place, .. }
            | ScalarExpression::CheckedAddress { place, .. } => self.place(place)?,
            ScalarExpression::Name { .. }
            | ScalarExpression::Member { .. }
            | ScalarExpression::Integer { .. }
            | ScalarExpression::InvalidInteger { .. }
            | ScalarExpression::Float { .. }
            | ScalarExpression::InvalidFloat { .. }
            | ScalarExpression::Boolean { .. }
            | ScalarExpression::Char { .. }
            | ScalarExpression::Utf8 { .. } => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

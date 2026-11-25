use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IntegerExpression {
    pub(crate) span: ByteSpan,
    pub(crate) ty: ScalarType,
    pub(crate) kind: IntegerKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IntegerKind {
    Literal(BigInt),
    Formal(ByteSpan),
    Payload(usize),
    Binary { operator: BinaryOperator, operands: Box<[IntegerExpression; 2]> },
}

pub(super) fn compile(
    source: &SourceIdentity, expression: &ScalarExpression, ty: &ScalarType,
    names: &impl Fn(&str) -> Option<(ScalarType, IntegerKind)>,
    work: &mut usize, depth: usize,
) -> Result<IntegerExpression, Diagnostic> {
    let span = expression_span(expression);
    let error = |message| diagnostic(source, span, message);
    if depth == analysis_budget::MAX_ACTUAL_CALL_DEPTH
        || analysis_budget::charge_call_work(work, 1).is_none()
    {
        return Err(error("runtime enum primitive expression exceeds analysis limits"));
    }
    if !is_integer_type(ty) { return Err(error("runtime enum expression requires an integer type")); }
    let kind = match expression {
        ScalarExpression::Name { name, .. } => {
            let (actual, binding) = names(name).ok_or_else(|| error("runtime enum expression requires an in-scope primitive binding"))?;
            if &actual != ty { return Err(error("runtime enum expression binding has a different integer type")); }
            binding
        }
        ScalarExpression::Binary { operator, left, right, .. }
            if matches!(operator, BinaryOperator::WrappingAdd | BinaryOperator::BitAnd | BinaryOperator::BitOr | BinaryOperator::BitXor) => {
                let left = compile(source, left, ty, names, work, depth + 1)?;
                let right = compile(source, right, ty, names, work, depth + 1)?;
                IntegerKind::Binary { operator: operator.clone(), operands: Box::new([left, right]) }
            }
        _ => {
            let value = concrete_integer_literal(expression)
                .ok_or_else(|| error("runtime enum expression requires a total integer operator, literal or primitive binding"))?;
            if !integer_literal_fits_type(&value, ty) { return Err(error("payload literal does not fit the declared integer type")); }
            IntegerKind::Literal(value.into_owned())
        }
    };
    Ok(IntegerExpression { span, ty: ty.clone(), kind })
}

pub(super) fn comparison_type(left: &ScalarExpression, right: &ScalarExpression, formals: &Formals,
    work: &mut usize,
) -> Option<ScalarType> {
    let mut pending = vec![right, left];
    while let Some(expression) = pending.pop() {
        analysis_budget::charge_call_work(work, 1)?;
        match expression {
            ScalarExpression::Name { name, .. } => {
                let (_, ty) = formals.get(name)?;
                return is_integer_type(ty).then(|| ty.clone());
            }
            ScalarExpression::Binary { left, right, .. } => {
                pending.push(right);
                pending.push(left);
            }
            _ => {}
        }
    }
    None
}

impl IntegerExpression {
    pub(super) fn for_each_formal(&self, visit: &mut impl FnMut(ByteSpan, ByteSpan)) {
        let mut pending = vec![self];
        while let Some(expr) = pending.pop() {
            match &expr.kind {
                IntegerKind::Formal(declaration) => visit(expr.span, *declaration),
                IntegerKind::Binary { operands, .. } => {
                    pending.push(&operands[1]);
                    pending.push(&operands[0]);
                }
                _ => {}
            }
        }
    }
}

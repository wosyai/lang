use super::*;

// The matcher shares the same typed goal syntax between flat and case
// scripts. Premises are supplied by the caller's preceding checked shows.
#[derive(Clone)]
pub(super) struct Goal<'a> {
    operator: &'a BinaryOperator,
    terms: [Term<'a>; 2],
}

#[derive(Clone, Eq, PartialEq)]
enum Term<'a> {
    Output,
    Literal(Cow<'a, BigInt>),
    Entry { old: bool },
}

pub(super) enum Premise<'a> {
    Literal(&'a BigInt),
    Entry,
}

pub(super) fn arguments(item: &ScalarBlockItem) -> Option<(&ScalarExpression, &ScalarExpression)> {
    let ScalarBlockItem::Expression(ScalarExpression::Call {
        receiver: Some(core), name, arguments, type_arguments, ..
    }) = item else { return None; };
    if core != "core" || name != "rewrite" || !type_arguments.is_empty() { return None; }
    let [equality, context] = arguments.as_slice() else { return None; };
    Some((equality, context))
}

impl<'a> Goal<'a> {
    pub(super) fn new(operator: &'a BinaryOperator, bound: &'a BigInt) -> Self {
        Self { operator, terms: [Term::Output, Term::Literal(Cow::Borrowed(bound))] }
    }

    /// Evaluate a normalized one-hole goal when the actual output is known
    /// and no entry-dependent term was admitted by the caller.
    pub(super) fn accepts_literal_output(&self, output: &BigInt) -> Result<bool, ProjectCheck> {
        fn value<'term>(term: &'term Term<'_>, output: &'term BigInt) -> Option<&'term BigInt> {
            match term {
                Term::Output => Some(output),
                Term::Literal(value) => Some(value.as_ref()),
                Term::Entry { .. } => None,
            }
        }
        let left = value(&self.terms[0], output).ok_or(ProjectCheck::Unsupported)?;
        let right = value(&self.terms[1], output).ok_or(ProjectCheck::Unsupported)?;
        let relation = IntegerRelation::from_operator(self.operator).ok_or(ProjectCheck::Unsupported)?;
        Ok(relation.accepts(left.cmp(right)))
    }

    pub(super) fn apply(
        &mut self, equality: &'a ScalarExpression, context: &'a ScalarExpression,
        output: &str, ty: &ScalarType, unary_entry: Option<&str>,
        established: impl FnOnce(Premise<'_>) -> Result<bool, ProjectCheck>,
    ) -> Result<(), ProjectCheck> {
        let ScalarExpression::Binary { operator: BinaryOperator::Equal, left, right, .. } = equality else {
            return Err(ProjectCheck::InvalidProof);
        };
        let term = |expression: &'a ScalarExpression| -> Option<Term<'a>> {
            match expression {
                ScalarExpression::Name { name, .. } if name == output => Some(Term::Output),
                ScalarExpression::Name { name, .. } if unary_entry == Some(name.as_str()) => Some(Term::Entry { old: false }),
                ScalarExpression::Call { receiver: Some(core), name, arguments, type_arguments, .. }
                    if core == "core" && name == "old" && type_arguments.is_empty()
                    && matches!(arguments.as_slice(), [ScalarExpression::Name { name, .. }] if unary_entry == Some(name.as_str())) =>
                {
                    Some(Term::Entry { old: true })
                }
                _ => {
                    let literal = concrete_integer_literal(expression)?;
                    integer_literal_fits_type(literal.as_ref(), ty).then_some(Term::Literal(literal))
                }
            }
        };
        let before = term(left).ok_or(ProjectCheck::Unsupported)?;
        let after = term(right).ok_or(ProjectCheck::Unsupported)?;
        let premise = match (&before, &after) {
            (Term::Output, Term::Literal(value)) | (Term::Literal(value), Term::Output) => Premise::Literal(value),
            (Term::Output, Term::Entry { .. }) | (Term::Entry { .. }, Term::Output) => Premise::Entry,
            _ => return Err(ProjectCheck::Unsupported),
        };
        if !established(premise)? { return Err(ProjectCheck::InvalidProof); }
        let ScalarExpression::Binary { operator, left, right, .. } = context else {
            return Err(ProjectCheck::InvalidProof);
        };
        if operator != self.operator { return Err(ProjectCheck::InvalidProof); }
        let hole = |expr: &ScalarExpression| matches!(expr, ScalarExpression::Name { name, .. } if name == "_");
        let (slot, other) = match (hole(left), hole(right)) {
            (true, false) => (0, term(right)),
            (false, true) => (1, term(left)),
            _ => return Err(ProjectCheck::InvalidProof),
        };
        if other.as_ref() != Some(&self.terms[1 - slot]) { return Err(ProjectCheck::InvalidProof); }
        self.terms[slot] = match (self.terms[slot] == before, self.terms[slot] == after) {
            (true, false) => after,
            (false, true) => before,
            _ => return Err(ProjectCheck::InvalidProof),
        };
        Ok(())
    }
}

use num_bigint::BigInt;

use super::{
    symbolic_integer::IntegerSet,
    typecheck::is_integer_type,
    BinaryOperator, ReferenceBindingId, ScalarExpression, ScalarType,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IntegerPredicate {
    pub(super) binding: ReferenceBindingId,
    ty: ScalarType,
    operator: BinaryOperator,
    bound: BigInt,
}

pub(super) struct IntegerComparison<'a> {
    pub(super) left: &'a ScalarExpression,
    pub(super) right: &'a ScalarExpression,
    pub(super) name: &'a str,
    operator: &'a BinaryOperator,
    bound: &'a BigInt,
}

impl<'a> IntegerComparison<'a> {
    pub(super) fn from_expression(expression: &'a ScalarExpression) -> Option<Self> {
        let ScalarExpression::Binary { operator, left, right, .. } = expression else {
            return None;
        };
        if !matches!(
            operator,
            BinaryOperator::Equal | BinaryOperator::NotEqual | BinaryOperator::Less
                | BinaryOperator::LessEqual | BinaryOperator::Greater | BinaryOperator::GreaterEqual
        ) {
            return None;
        }
        let ScalarExpression::Name { name, .. } = left.as_ref() else {
            return None;
        };
        let ScalarExpression::Integer { value: bound, .. } = right.as_ref() else {
            return None;
        };
        Some(Self { left, right, name, operator, bound })
    }

    pub(super) fn typed(
        &self,
        left_type: &ScalarType,
        right_type: &ScalarType,
        binding: ReferenceBindingId,
    ) -> Option<IntegerPredicate> {
        if left_type != right_type || !is_integer_type(left_type) {
            return None;
        }
        Some(IntegerPredicate {
            binding,
            ty: left_type.clone(),
            operator: self.operator.clone(),
            bound: self.bound.clone(),
        })
    }
}

impl IntegerPredicate {
    pub(super) fn root(&self) -> Option<IntegerPath> {
        Some(IntegerPath {
            binding: self.binding.clone(),
            ty: self.ty.clone(),
            values: IntegerSet::from_type(&self.ty)?,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct IntegerPath {
    binding: ReferenceBindingId,
    ty: ScalarType,
    values: IntegerSet,
}

impl IntegerPath {
    pub(super) fn split(&self, predicate: &IntegerPredicate) -> Option<(Self, Self)> {
        if self.binding != predicate.binding || self.ty != predicate.ty {
            return None;
        }
        let (yes, no) = self.values.split(&predicate.operator, &predicate.bound)?;
        let path = |values| Self { binding: self.binding.clone(), ty: self.ty.clone(), values };
        Some((path(yes), path(no)))
    }

    pub(super) fn join(self, other: Self) -> Option<Self> {
        if self.binding != other.binding || self.ty != other.ty {
            return None;
        }
        Some(Self { values: self.values.join(other.values), ..self })
    }

    pub(super) fn is_unreachable(&self) -> bool {
        self.values.is_empty()
    }
}

use std::{collections::HashMap, rc::Rc};

use num_bigint::BigInt;

use super::{
    checker_directive::CheckerOperation,
    control::{BlockItemKind, ConditionalRegion},
    reference::{BlockOrdinal, FunctionCallSites, OccurrenceOrdinal, PlainNameIdentity},
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

type IntegerRow = Vec<Rc<IntegerSet>>;
type IntegerRoots<'a> = HashMap<&'a ReferenceBindingId, (usize, &'a ScalarType)>;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct IntegerInputs(Vec<IntegerRow>);

impl IntegerInputs {
    fn split(
        self,
        roots: &IntegerRoots<'_>,
        predicate: &IntegerPredicate,
    ) -> Option<(Self, Self)> {
        let &(index, ty) = roots.get(&predicate.binding)?;
        if ty != &predicate.ty {
            return None;
        }
        let mut passing = Self::default();
        let mut failing = Self::default();
        for mut row in self.0 {
            let (yes, no) = row[index].split(&predicate.operator, &predicate.bound)?;
            if yes.is_empty() {
                failing.0.push(row);
            } else if no.is_empty() {
                passing.0.push(row);
            } else {
                let mut alternate = row.clone();
                row[index] = Rc::new(yes);
                alternate[index] = Rc::new(no);
                passing.0.push(row);
                failing.0.push(alternate);
            }
        }
        Some((passing, failing))
    }

    fn join(mut self, other: Self) -> Self {
        self.0.extend(other.0);
        self
    }

    fn is_unreachable(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn includes(&self, coordinates: &[u128]) -> bool {
        self.0.iter().any(|row| {
            row.len() == coordinates.len()
                && row.iter().zip(coordinates).all(|(axis, value)| {
                    axis.split(&BinaryOperator::Equal, &BigInt::from(*value))
                        .is_some_and(|(matching, _)| !matching.is_empty())
                })
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct IntegerCheckFailure {
    pub(super) directive: OccurrenceOrdinal,
    pub(super) inputs: IntegerInputs,
}

enum IntegerControl {
    Continue(IntegerInputs),
    Fail(IntegerCheckFailure),
}

pub(super) fn check_integer_requirements(
    calls: &FunctionCallSites,
) -> Option<Result<(), IntegerCheckFailure>> {
    let boundary = &calls.boundary;
    if boundary.parameters.is_empty()
        || boundary.parameters.len() != boundary.checker_parameter_bindings.len()
    {
        return None;
    }
    let roots: IntegerRoots<'_> = boundary
        .checker_parameter_bindings
        .iter()
        .zip(&boundary.parameters)
        .enumerate()
        .map(|(index, (binding, parameter))| (binding, (index, &parameter.ty)))
        .collect();
    let root = boundary
        .parameters
        .iter()
        .map(|parameter| IntegerSet::from_type(&parameter.ty).map(Rc::new))
        .collect::<Option<IntegerRow>>()?;
    let root = IntegerInputs(vec![root]);
    match walk_integer_block(calls, &roots, calls.structure.root, root)? {
        IntegerControl::Continue(_) => Some(Ok(())),
        IntegerControl::Fail(failure) => Some(Err(failure)),
    }
}

fn walk_integer_block(
    calls: &FunctionCallSites,
    roots: &IntegerRoots<'_>,
    block: BlockOrdinal,
    mut path: IntegerInputs,
) -> Option<IntegerControl> {
    let structure = &calls.structure;
    for item in &structure.blocks.get(block.0)?.items {
        if path.is_unreachable() {
            break;
        }
        match item.kind {
            BlockItemKind::Evaluation { expression } => {
                if let Some(directive) = structure.checker_directives.get(&expression) {
                    let (passing, failing) = path.split(roots, directive.integer_predicate.as_ref()?)?;
                    match directive.operation {
                        CheckerOperation::Require if !failing.is_unreachable() => {
                            return Some(IntegerControl::Fail(IntegerCheckFailure {
                                directive: expression,
                                inputs: failing,
                            }));
                        }
                        CheckerOperation::Narrow if passing.is_unreachable() => {
                            return Some(IntegerControl::Fail(IntegerCheckFailure {
                                directive: expression,
                                inputs: failing,
                            }));
                        }
                        CheckerOperation::Require | CheckerOperation::Narrow => path = passing,
                    }
                } else {
                    let ConditionalRegion::Unit { condition, then_block, .. } =
                        calls.conditionals.get(&expression)?
                    else {
                        return None;
                    };
                    let predicate = structure.checker_integer_branches.get(condition)?;
                    let (yes, no) = path.split(roots, predicate)?;
                    match walk_integer_block(calls, roots, *then_block, yes)? {
                        IntegerControl::Continue(then_path) => path = then_path.join(no),
                        failure @ IntegerControl::Fail(_) => return Some(failure),
                    }
                }
            }
            BlockItemKind::FinalOutput(slot) => {
                let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(slot.expression)?
                else {
                    return None;
                };
                if !roots.contains_key(binding) {
                    return None;
                }
            }
            BlockItemKind::Binding(_) | BlockItemKind::Assignment { .. } => return None,
        }
    }
    Some(IntegerControl::Continue(path))
}

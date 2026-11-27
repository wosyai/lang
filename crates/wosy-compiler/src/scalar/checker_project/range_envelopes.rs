use super::*;
use crate::scalar::reference::{BlockOrdinal, OccurrenceEdge, OccurrenceNode, OccurrenceParent, PlainNameIdentity};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Envelope {
    pub(super) low: BigInt,
    pub(super) high: BigInt,
}

impl Envelope {
    fn whole(ty: &ScalarType) -> Option<Self> {
        let (low, high) = super::super::typecheck::integer_bounds(ty)?;
        Some(Self { low, high })
    }

    fn exact(value: BigInt) -> Self {
        Self { low: value.clone(), high: value }
    }

    fn hull(self, rhs: Self) -> Self {
        Self { low: self.low.min(rhs.low), high: self.high.max(rhs.high) }
    }

    fn fit(self, ty: &ScalarType) -> Option<Self> {
        let full = Self::whole(ty)?;
        Some(if self.low < full.low || self.high > full.high { full } else { self })
    }

    fn binary(self, op: &super::super::BinaryOperator, rhs: Self, ty: &ScalarType) -> Option<Self> {
        use super::super::BinaryOperator::*;
        let bounds = match op {
            Add => Self { low: self.low + rhs.low, high: self.high + rhs.high },
            Subtract => Self { low: self.low - rhs.high, high: self.high - rhs.low },
            Multiply if self.low == self.high && rhs.low == rhs.high => Self::exact(self.low * rhs.low),
            Divide if self.low == self.high && rhs.low == rhs.high && rhs.low.bits() != 0 => Self::exact(self.low / rhs.low),
            Remainder if self.low == self.high && rhs.low == rhs.high && rhs.low.bits() != 0 => Self::exact(self.low % rhs.low),
            Multiply | Divide | Remainder => Self::whole(ty)?,
            _ => return None,
        };
        bounds.fit(ty)
    }
}

type Inputs = Vec<Option<Envelope>>;
type Key = (PreparedCallable, Inputs);

struct Frame<'a> {
    body: &'a FunctionCallSites,
    target: PreparedCallable,
    ids: &'a [ReferenceBindingId],
}

impl<'a> Frame<'a> {
    fn formal(&self, e: OccurrenceOrdinal) -> Option<usize> {
        let PlainNameIdentity::Lexical(binding) = self.body.prepared_plain_name(e)? else { return None; };
        self.ids.iter().position(|formal| formal == binding)
    }

    fn output(&self, block: BlockOrdinal) -> Option<OccurrenceOrdinal> {
        let [item] = self.body.structure.blocks.get(block.0)?.items.as_slice() else { return None; };
        let BlockItemKind::FinalOutput(slot) = item.kind else { return None; };
        (slot.output == 0).then_some(slot.expression)
    }

    fn conditional(&self, e: OccurrenceOrdinal) -> Option<(OccurrenceOrdinal, OccurrenceOrdinal, OccurrenceOrdinal)> {
        let ConditionalRegion::Value { condition, then_block, else_block, resume, .. } = self.body.conditionals.get(&e)? else { return None; };
        if *resume != e || self.body.structure.checked_types.expressions.get(condition)? != &ScalarType::Bool {
            return None;
        }
        Some((*condition, self.output(*then_block)?, self.output(*else_block)?))
    }
}

pub(super) struct PreparedEnvelopes<'a> {
    project: &'a ScalarProject,
    calls: &'a ProjectCallSiteResults,
    integer: HashMap<Key, Envelope>,
    active: HashSet<PreparedCallable>,
}

impl<'a> PreparedEnvelopes<'a> {
    pub(super) fn new(project: &'a ScalarProject, calls: &'a ProjectCallSiteResults) -> Self {
        Self { project, calls, integer: HashMap::new(), active: HashSet::new() }
    }

    fn frame(&self, target: PreparedCallable) -> Option<Frame<'a>> {
        self.frame_with_obligations(target,false)
    }

    fn frame_with_obligations(&self, target: PreparedCallable, authored: bool) -> Option<Frame<'a>> {
        let function = self.calls.function_body(target.0, target.1, None)?;
        if function.contextual_formal().is_some() || !function.arms.is_empty() || !function.assignments.is_empty()
            || !function.callable_escapes.is_empty() || !function.allocation_length_calls.is_empty()
            || !function.structure.module_storage_uses.is_empty()
            || (!authored && !function.structure.checker_directives.is_empty())
            || function.structure.blocks.iter().any(|block| block.unsafe_context)
        { return None; }
        let ids = function.boundary.checker_parameter_bindings.as_deref()?;
        if ids.len() != function.boundary.parameters.len() { return None; }
        Some(Frame { body: function, target, ids })
    }

    fn arguments(&mut self, frame: &Frame<'a>, e: OccurrenceOrdinal, inputs: &Inputs) -> Option<Key> {
        let signature = frame.body.checked_call_signatures.get(&e)?;
        let child = super::authenticate_prepared_direct_edge(self.project,self.calls,frame.target.0,frame.body,e,signature)?;
        let auth = self.frame(child)?;
        if !auth.body.boundary.parameters.iter().map(|p| &p.ty).eq(signature.parameters.iter())
            || !auth.body.boundary.results.iter().map(|r| &r.ty).eq(signature.outputs.outputs.iter().map(|r| &r.ty))
        { return None; }
        let mut args = Vec::with_capacity(signature.parameters.len());
        for (slot, ty) in signature.parameters.iter().enumerate() {
            let OccurrenceNode::Expression(argument) = frame.body.structure.edges.get(&(
                OccurrenceParent::Expression(e), OccurrenceEdge::CallArgument(slot)
            ))? else { return None; };
            if frame.body.structure.checked_types.expressions.get(argument)? != ty { return None; }
            args.push(if is_integer_type(ty) {
                Some(self.value(frame, *argument, inputs)?)
            } else if *ty == ScalarType::Bool && (frame.formal(*argument).is_some()
                || frame.body.structure.observed_bool_literals.contains_key(argument)) {
                None
            } else { return None; });
        }
        Some((child,args))
    }

    fn value(&mut self, frame: &Frame<'a>, e: OccurrenceOrdinal, inputs: &Inputs) -> Option<Envelope> {
        let structure = &frame.body.structure;
        let ty = structure.checked_types.expressions.get(&e)?;
        if !is_integer_type(ty) { return None; }
        if let Some(value) = structure.prepared_range_literals.get(&e) {
            return super::super::typecheck::integer_literal_fits_type(value,ty).then(|| Envelope::exact(value.clone()));
        }
        if let Some(idx) = frame.formal(e) { return inputs.get(idx)?.clone(); }
        if let Some((left,op,right)) = structure.prepared_range_binary.get(&e) {
            return self.value(frame,*left,inputs)?.binary(op,self.value(frame,*right,inputs)?,ty);
        }
        if let Some((condition, yes, no)) = frame.conditional(e) {
            // Recognize the source relation of min/max, not its spelling.
            // Both branch outputs must be the same two prepared operands.
            if let Some((left,op,right)) = structure.prepared_range_binary.get(&condition) {
                use super::super::BinaryOperator::*;
                if let (Some(lhs),Some(rhs),Some(success),Some(failure)) =
                    (frame.formal(*left), frame.formal(*right), frame.formal(yes), frame.formal(no))
                {
                    let picks_left_on_true = (success,failure) == (lhs,rhs);
                    let picks_right_on_true = (success,failure) == (rhs,lhs);
                    if picks_left_on_true || picks_right_on_true {
                        let max = match op {
                            Less | LessEqual => Some(picks_right_on_true),
                            Greater | GreaterEqual => Some(picks_left_on_true),
                            _ => None,
                        };
                        if let Some(max) = max {
                            let (l,r) = (inputs.get(lhs)?.as_ref()?, inputs.get(rhs)?.as_ref()?);
                            let pick = if max { std::cmp::max } else { std::cmp::min };
                            return Some(Envelope { low: pick(l.low.clone(),r.low.clone()), high: pick(l.high.clone(),r.high.clone()) });
                        }
                    }
                }
            }
            let left = self.facts(frame, condition, true, inputs.clone())?;
            let right = self.facts(frame, condition, false, inputs.clone())?;
            return match (left,right) {
                (Some(l),Some(r)) => Some(self.value(frame,yes,&l)?.hull(self.value(frame,no,&r)?)),
                (Some(l),None) => self.value(frame,yes,&l),
                (None,Some(r)) => self.value(frame,no,&r),
                (None,None) => Envelope::whole(ty),
            };
        }
        let key = self.arguments(frame,e,inputs)?;
        if let Some(value) = self.integer.get(&key) { return Some(value.clone()); }
        if !self.active.insert(key.0) { return None; }
        let evaluated = (|| {
            let child = self.frame(key.0)?;
            self.value(&child, child.output(child.body.structure.root)?, &key.1)
        })();
        self.active.remove(&key.0);
        let value = evaluated?;
        self.integer.insert(key,value.clone());
        Some(value)
    }

    fn facts(&mut self, frame: &Frame<'a>, e: OccurrenceOrdinal, want: bool, mut input: Inputs) -> Option<Option<Inputs>> {
        let structure = &frame.body.structure;
        if structure.checked_types.expressions.get(&e)? != &ScalarType::Bool { return None; }
        if let Some(value) = structure.observed_bool_literals.get(&e) { return Some((want == *value).then_some(input)); }
        if frame.formal(e).is_some() { return Some(Some(input)); }
        if let Some(inner) = structure.observed_bool_negations.get(&e) { return self.facts(frame,*inner,!want,input); }
        if let Some(branch) = frame.body.short_circuits.get(&e) {
            let continuing = branch.evaluate_right_when;
            if want == continuing {
                let Some(left) = self.facts(frame,branch.left,continuing,input)? else { return Some(None); };
                return self.facts(frame,branch.right,want,left);
            }
            let bypass = self.facts(frame,branch.left,!continuing,input.clone())?;
            let via_right = match self.facts(frame,branch.left,continuing,input)? {
                Some(left) => self.facts(frame,branch.right,want,left)?,
                None => None,
            };
            return Some(join(bypass,via_right));
        }
        if let Some((condition, yes, no)) = frame.conditional(e) {
            let yes = match self.facts(frame,condition,true,input.clone())? {
                Some(row) => self.facts(frame,yes,want,row)?,
                None => None,
            };
            let no = match self.facts(frame,condition,false,input)? {
                Some(row) => self.facts(frame,no,want,row)?,
                None => None,
            };
            return Some(join(yes,no));
        }
        let Some((left,op,right)) = structure.prepared_range_binary.get(&e) else {
            let key = self.arguments(frame,e,&input)?;
            if !self.active.insert(key.0) { return None; }
            let evaluated = (|| {
                let child = self.frame(key.0)?;
                self.facts(&child,child.output(child.body.structure.root)?,want,key.1)
            })();
            self.active.remove(&key.0);
            let Some(row) = evaluated? else { return Some(None); };
            for (slot,bounds) in row.into_iter().enumerate() {
                let Some(bounds) = bounds else { continue; };
                let OccurrenceNode::Expression(arg) = structure.edges.get(&(
                    OccurrenceParent::Expression(e), OccurrenceEdge::CallArgument(slot),
                ))? else { return None; };
                if let Some(index) = frame.formal(*arg) {
                    let current = input.get_mut(index)?.as_mut()?;
                    current.low = std::cmp::max(current.low.clone(),bounds.low);
                    current.high = std::cmp::min(current.high.clone(),bounds.high);
                    if current.low > current.high { return Some(None); }
                }
            }
            return Some(Some(input));
        };
        let right_bounds = self.value(frame,*right,&input)?;
        let left_bounds = self.value(frame,*left,&input)?;
        if let Some(index) = frame.formal(*left) {
            let bound = input.get_mut(index)?.as_mut()?;
            intersect_comparison(bound, op, want, &right_bounds, false)?;
            if bound.low > bound.high { return Some(None); }
        }
        if let Some(index) = frame.formal(*right) {
            let bound = input.get_mut(index)?.as_mut()?;
            intersect_comparison(bound, op, want, &left_bounds, true)?;
            if bound.low > bound.high { return Some(None); }
        }
        Some(Some(input))
    }

    /// Close only obligations universal under exact same-call branch facts.
    /// Every remaining authored site and any unsupported history use the
    /// existing search; proving one call site never certifies other callers.
    pub(super) fn close_guarded_formals(&mut self, target: PreparedCallable) -> Option<Vec<ByteSpan>> {
        use crate::scalar::reference::{CheckerOrdinaryBoolean, NamedPlaceRootIdentity};
        let frame = self.frame_with_obligations(target,true)?;
        let b = frame.body;
        let st = &b.structure;
        if st.checker_directives.is_empty() || !b.checker_returns_unit() || b.conditionals.is_empty() {
            return None;
        }
        if (0..b.occurrences.len()).any(|at| matches!(b.prepared_named_place_root(OccurrenceOrdinal(at)),
            Some(NamedPlaceRootIdentity::Lexical(binding)) if frame.ids.contains(binding))) {
            return None;
        }
        let init = initial_inputs(b)?;
        let mut blocks = vec![(st.root,init)];
        let mut covered = Vec::new();
        while let Some((block,inputs)) = blocks.pop() {
            for item in &st.blocks.get(block.0)?.items {
                let BlockItemKind::Evaluation { expression } = item.kind else { continue; };
                if let Some(directive) = st.checker_directives.get(&expression) {
                    if directive.operation != CheckerOperation::Require { return None; }
                    let accepts = |c: &super::super::checker_flow::IntegerPredicate| {
                        let axis = frame.ids.iter().position(|id| id == &c.binding)?;
                        let range = inputs.get(axis)?.as_ref()?;
                        if b.boundary.parameters.get(axis)?.ty != c.ty { return None; }
                        IntegerRelation::from_operator(&c.operator)?
                            .accepts_closed(&range.low,&range.high,&c.bound).then_some(())
                    };
                    match directive.predicate.as_ref() {
                        Some(CheckerPredicate::Integer(c)) => accepts(c)?,
                        None => {
                            let mut todo = vec![st.checker_ordinary_booleans.get(&expression)?];
                            while let Some(p) = todo.pop() {
                                match p {
                                    CheckerOrdinaryBoolean::Integer(c) => accepts(c)?,
                                    CheckerOrdinaryBoolean::And(pair) => todo.extend(pair.iter().rev()),
                                    _ => return None,
                                }
                            }
                        }
                        _ => return None,
                    }
                    covered.push(b.occurrences.get(expression.0)?.span);
                    continue;
                }
                let (condition, yes, no) = match b.conditionals.get(&expression) {
                    Some(ConditionalRegion::Unit { condition, then_block, .. }) => (*condition,*then_block,None),
                    Some(ConditionalRegion::Value { condition, then_block, else_block, .. })
                        if st.checked_types.expressions.get(&expression) == Some(&ScalarType::Unit) =>
                        (*condition,*then_block,Some(*else_block)),
                    _ => continue,
                };
                // A semantic unknown is not a narrow. Do not certify either
                // arm on that test. No state is imported from another caller.
                if let Some(row) = self.facts(&frame,condition,true,inputs.clone())? { blocks.push((yes,row)); }
                if let Some(no) = no {
                    if let Some(row) = self.facts(&frame,condition,false,inputs.clone())? { blocks.push((no,row)); }
                }
            }
        }
        (covered.len() == st.checker_directives.len()).then_some(covered)
    }

    #[cfg(test)]
    pub(super) fn true_formal_envelopes(&mut self, target: PreparedCallable) -> Option<Inputs> {
        let frame = self.frame(target)?;
        if !matches!(frame.body.boundary.results.as_slice(), [result] if result.ty == ScalarType::Bool) { return None; }
        self.facts(&frame, frame.output(frame.body.structure.root)?, true, initial_inputs(frame.body)?)?
    }
}

fn initial_inputs(body: &FunctionCallSites) -> Option<Inputs> {
    body.boundary.parameters.iter().map(|p| if p.ty == ScalarType::Bool { Some(None) }
        else { Envelope::whole(&p.ty).map(Some) }).collect()
}

fn join(left: Option<Inputs>, right: Option<Inputs>) -> Option<Inputs> {
    match (left,right) {
        (Some(left),Some(right)) => Some(left.into_iter().zip(right).map(|(l,r)| match (l,r) {
            (Some(l),Some(r)) => Some(l.hull(r)),
            _ => None,
        }).collect()),
        (left,right) => left.or(right),
    }
}

fn intersect_comparison(
    target: &mut Envelope, op: &super::super::BinaryOperator, want: bool, other: &Envelope, reverse: bool,
) -> Option<()> {
    use super::super::BinaryOperator::*;
    if let Some((hi, strict)) = match (op,want,reverse) {
        (Less,true,false) | (LessEqual,false,true) | (Greater,true,true) | (GreaterEqual,false,false) => Some((true,true)),
        (LessEqual,true,false) | (Less,false,true) | (GreaterEqual,true,true) | (Greater,false,false) => Some((true,false)),
        (Greater,true,false) | (GreaterEqual,false,true) | (Less,true,true) | (LessEqual,false,false) => Some((false,true)),
        (GreaterEqual,true,false) | (Greater,false,true) | (LessEqual,true,true) | (Less,false,false) => Some((false,false)),
        (Equal,true,_) | (NotEqual,false,_) => {
            target.low = std::cmp::max(target.low.clone(),other.low.clone());
            target.high = std::cmp::min(target.high.clone(),other.high.clone());
            None
        }
        (Equal,false,_) | (NotEqual,true,_) => None,
        _ => return None,
    } {
        if hi {
            target.high = std::cmp::min(target.high.clone(), &other.high - u8::from(strict));
        } else {
            target.low = std::cmp::max(target.low.clone(), &other.low + u8::from(strict));
        }
    }
    Some(())
}
